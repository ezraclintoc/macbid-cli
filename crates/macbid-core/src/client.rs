//! Main HTTP client.
//!
//! Two upstreams, both read-only:
//!
//! * `https://search-1..5.mac.bid` — the Typesense index the Mac.Bid web app
//!   searches through. Facets, ordering and pagination all live here.
//! * `https://api.macdiscount.com` — the storefront API; `/lot/<id>` serves a
//!   lot's description, test notes and photos (no auth required).
//!
//! The search key is the read-only key the public web bundle ships. It can be
//! overridden with `MACBID_TYPESENSE_KEY` (and the nodes with
//! `MACBID_TYPESENSE_NODES`) if Mac.Bid ever rotates it.

use crate::error::{Error, Result};
use crate::models::{FacetField, Lot, LotDetail, SearchFilter, SearchResponse};
use reqwest::Client;
use std::time::Duration;

/// Search nodes, tried in order until one answers.
pub const DEFAULT_NODES: &[&str] = &[
    "https://search-1.mac.bid",
    "https://search-2.mac.bid",
    "https://search-3.mac.bid",
    "https://search-4.mac.bid",
    "https://search-5.mac.bid",
];

/// Public read-only search key (shipped in the mac.bid web bundle).
pub const DEFAULT_KEY: &str = "jxX8RU6YVOkm9esgd9buaYjulIWv6N52";

/// Typesense collection backing mac.bid search.
pub const DEFAULT_COLLECTION: &str = "prod_macdiscount_alias";

/// Storefront API base (lot details, no auth).
pub const DEFAULT_API_BASE: &str = "https://api.macdiscount.com";

/// Async client for Mac.Bid's search index.
#[derive(Debug, Clone)]
pub struct MacBidClient {
    http: Client,
    nodes: Vec<String>,
    key: String,
    collection: String,
    api_base: String,
}

impl MacBidClient {
    /// Build a client from the defaults, honoring the `MACBID_*` env vars.
    pub fn new() -> Result<Self> {
        let nodes = match std::env::var("MACBID_TYPESENSE_NODES") {
            Ok(v) if !v.trim().is_empty() => v
                .split(',')
                .map(|n| n.trim().trim_end_matches('/').to_string())
                .filter(|n| !n.is_empty())
                .collect(),
            _ => DEFAULT_NODES.iter().map(|n| n.to_string()).collect(),
        };
        let key = std::env::var("MACBID_TYPESENSE_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_KEY.to_string());
        let collection = std::env::var("MACBID_TYPESENSE_COLLECTION")
            .ok()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
        let api_base = std::env::var("MACBID_API_BASE")
            .ok()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());

        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(8))
            .user_agent("macbid-cli (read-only; github.com/ezraclintoc/macbid-cli)")
            .build()?;

        Ok(Self {
            http,
            nodes,
            key,
            collection,
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }

    /// Override the storefront API base (used by tests).
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into().trim_end_matches('/').to_string();
        self
    }

    /// Override the search nodes (used by tests).
    pub fn with_nodes(mut self, nodes: Vec<String>) -> Self {
        if !nodes.is_empty() {
            self.nodes = nodes;
        }
        self
    }

    /// Run a search, with Mac.Bid's own facets, ordering and pagination.
    pub async fn search(&self, filter: &SearchFilter) -> Result<SearchResponse> {
        let pairs = filter.to_query_pairs();
        let mut last_err: Option<Error> = None;

        for node in &self.nodes {
            match self.search_on(node, &pairs).await {
                Ok(resp) => return Ok(resp),
                // A 4xx is our fault, not the node's: don't burn the other
                // nodes on it.
                Err(e @ Error::Api { status, .. }) if (400..500).contains(&status) => {
                    return Err(e);
                }
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            Error::Invalid("no search nodes configured (check MACBID_TYPESENSE_NODES)".into())
        }))
    }

    async fn search_on(&self, node: &str, pairs: &[(String, String)]) -> Result<SearchResponse> {
        let url = format!("{node}/collections/{}/documents/search", self.collection);
        let resp = self
            .http
            .get(&url)
            .query(&[("x-typesense-api-key", self.key.as_str())])
            .query(pairs)
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if status != 200 {
            return Err(Error::Api {
                status,
                url,
                message: api_message(&body),
            });
        }
        serde_json::from_str::<SearchResponse>(&body).map_err(Error::Json)
    }

    /// Fetch up to `pages` consecutive pages, merging and de-duplicating lots.
    ///
    /// Stops early when the index reports no further hits or returns an empty
    /// page, and pauses briefly between requests to stay polite.
    pub async fn search_pages(&self, filter: &SearchFilter, pages: u32) -> Result<MergedSearch> {
        let mut all: Vec<Lot> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut pages_fetched = 0u32;
        let mut total_found = 0i64;

        for page in 1..=pages.max(1) {
            let mut page_filter = filter.clone();
            page_filter.page = Some(page);
            let resp = self.search(&page_filter).await?;

            total_found = resp.found;
            let empty = resp.hits.is_empty();
            for lot in resp.lots() {
                if seen.insert(lot.lot_id) {
                    all.push(lot);
                }
            }
            pages_fetched += 1;

            let per_page = filter.per_page.unwrap_or(10).max(1) as i64;
            if empty || (page as i64) * per_page >= total_found {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        Ok(MergedSearch {
            response: SearchResponse {
                found: total_found,
                out_of: total_found,
                page: 1,
                hits: Vec::new(),
                facet_counts: Vec::new(),
            },
            lots: all,
            pages_fetched,
            total_found,
        })
    }

    /// Compute facet counts for `fields` (category, auction_location, …).
    pub async fn facets(&self, filter: &SearchFilter) -> Result<Facets> {
        let mut filter = filter.clone();
        filter.page = Some(1);
        filter.per_page = Some(1);
        if filter.facet_by.is_empty() {
            filter.facet_by = default_facet_fields();
        }
        let resp = self.search(&filter).await?;
        Ok(Facets {
            fields: resp.facet_counts,
            total_found: resp.found,
        })
    }

    /// Category names with counts, alphabetically.
    pub async fn categories(&self) -> Result<Vec<(String, i64)>> {
        self.facet_values("category").await
    }

    /// Auction locations with counts, busiest first.
    pub async fn locations(&self) -> Result<Vec<(String, i64)>> {
        self.facet_values("auction_location").await
    }

    /// Values of one facet field, sorted by count (desc) then value.
    pub async fn facet_values(&self, field: &str) -> Result<Vec<(String, i64)>> {
        self.facet_values_with(field, &SearchFilter::default())
            .await
    }

    /// Values of one facet field within `base`'s filters (e.g. the categories
    /// inside one category), sorted by count (desc) then value.
    pub async fn facet_values_with(
        &self,
        field: &str,
        base: &SearchFilter,
    ) -> Result<Vec<(String, i64)>> {
        let mut filter = base.clone();
        filter.facet_by = vec![field.to_string()];
        let facets = self.facets(&filter).await?;
        let mut values: Vec<(String, i64)> = facets
            .fields
            .into_iter()
            .find(|f| f.field_name == field)
            .map(|f| f.counts.into_iter().map(|c| (c.value, c.count)).collect())
            .unwrap_or_default();
        values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(values)
    }

    /// Full lot metadata: description, test notes, photos, condition.
    ///
    /// The lot payload has no current bid, so the search index is consulted
    /// too (best effort — a search hiccup leaves `current_bid` empty rather
    /// than failing the whole lookup).
    pub async fn lot(&self, lot_id: i64) -> Result<LotDetail> {
        let url = format!("{}/lot/{lot_id}", self.api_base);
        let resp = self.http.get(&url).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;

        if status == 404 {
            return Err(Error::NotFound(format!("lot {lot_id}")));
        }
        if status != 200 {
            return Err(Error::Api {
                status,
                url,
                message: api_message(&body),
            });
        }

        let mut detail: LotDetail = serde_json::from_str(&body)?;
        if detail.current_bid.is_none()
            && let Ok(Some(lot)) = self.lot_from_index(lot_id).await
        {
            detail.current_bid = Some(lot.current_bid);
            if detail.total_bids == 0 {
                detail.total_bids = lot.total_bids;
            }
            if detail.unique_bidders == 0 {
                detail.unique_bidders = lot.unique_bidders;
            }
        }
        Ok(detail)
    }

    /// Look a lot up in the search index by id (returns `None` when it is not
    /// indexed, e.g. a closed lot).
    pub async fn lot_from_index(&self, lot_id: i64) -> Result<Option<Lot>> {
        // The id filter is not expressible through SearchFilter (it is a
        // document key, not a facet), so the request is built by hand.
        let pairs = vec![
            ("q".to_string(), "*".to_string()),
            ("query_by".to_string(), "product_name".to_string()),
            ("filter_by".to_string(), format!("id:=`mac_lot_{lot_id}`")),
            ("per_page".to_string(), "1".to_string()),
            (
                "exclude_fields".to_string(),
                "embedding,images,keywords,bid_delta,description".to_string(),
            ),
        ];

        let mut last_err: Option<Error> = None;
        for node in &self.nodes {
            match self.search_on(node, &pairs).await {
                Ok(resp) => return Ok(resp.hits.into_iter().next().map(|h| h.document)),
                Err(e @ Error::Api { status, .. }) if (400..500).contains(&status) => {
                    return Err(e);
                }
                Err(e) => last_err = Some(e),
            }
        }
        match last_err {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }
}

/// Result of [`MacBidClient::search_pages`].
#[derive(Debug, Clone)]
pub struct MergedSearch {
    /// Envelope with counts (hits live in `lots`).
    pub response: SearchResponse,
    /// Merged, de-duplicated lots across fetched pages.
    pub lots: Vec<Lot>,
    /// How many pages were actually fetched.
    pub pages_fetched: u32,
    /// Total hits the index reports for the filter.
    pub total_found: i64,
}

/// Facet counts for one query.
#[derive(Debug, Clone)]
pub struct Facets {
    /// One entry per requested field.
    pub fields: Vec<FacetField>,
    /// Total documents matching the filter.
    pub total_found: i64,
}

/// Facet fields exposed by the index.
pub fn default_facet_fields() -> Vec<String> {
    ["category", "auction_location", "condition", "us_state"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Pull a human message out of an error body (Typesense uses `message`,
/// the storefront API sometimes uses `error`).
fn api_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        for key in ["message", "error", "detail"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                return s.to_string();
            }
        }
    }
    body.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_message_prefers_the_message_field() {
        assert_eq!(
            api_message(r#"{"message":"Could not parse the filter query."}"#),
            "Could not parse the filter query."
        );
        assert_eq!(api_message("plain text"), "plain text");
    }

    #[test]
    fn default_facet_fields_cover_the_site_facets() {
        let fields = default_facet_fields();
        assert!(fields.contains(&"category".to_string()));
        assert!(fields.contains(&"auction_location".to_string()));
    }
}
