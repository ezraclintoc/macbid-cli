//! Data models mirroring Mac.Bid's Typesense documents and lot-detail payload.
//!
//! Responses are deserialized leniently: unknown fields are ignored, numbers
//! and booleans accept the several shapes the backend emits (`1` / `true`,
//! `"3"` / `3`), so a schema drift upstream doesn't break the crate.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// filters

/// Physical condition of a lot. Mac.Bid only ever uses these three values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    /// `OPEN BOX`
    OpenBox,
    /// `LIKE NEW`
    LikeNew,
    /// `DAMAGED`
    Damaged,
}

impl Condition {
    /// The exact facet value used by the API.
    pub fn api_value(self) -> &'static str {
        match self {
            Self::OpenBox => "OPEN BOX",
            Self::LikeNew => "LIKE NEW",
            Self::Damaged => "DAMAGED",
        }
    }

    /// Parse user input tolerantly (`damaged`, `Like New`, `open_box`, …).
    pub fn parse(s: &str) -> Option<Self> {
        let norm = s.trim().to_lowercase().replace(['_', '-'], " ");
        match norm.as_str() {
            "open box" | "openbox" => Some(Self::OpenBox),
            "like new" | "likenew" => Some(Self::LikeNew),
            "damaged" => Some(Self::Damaged),
            _ => None,
        }
    }
}

/// Result ordering. Each variant maps to a `sort_by` clause the site itself
/// uses, so ordering is server-side (no client-side resorting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortOrder {
    /// `_text_match:desc,ranking_weight:desc` — Mac.Bid's default relevance.
    #[default]
    Relevance,
    /// `expected_closing_utc:asc` — lots closing first.
    EndingSoon,
    /// `launch_date_utc:desc` — freshly listed lots.
    Newest,
    /// `current_bid:asc`
    PriceLowToHigh,
    /// `current_bid:desc`
    PriceHighToLow,
    /// `retail_price:desc` — biggest retail value first.
    RetailHighToLow,
    /// `discount_percentage:desc` — deepest markdown first.
    BiggestDiscount,
    /// `total_bids:desc` — hottest lots first.
    MostBids,
}

impl SortOrder {
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Relevance => "_text_match:desc,ranking_weight:desc",
            Self::EndingSoon => "expected_closing_utc:asc",
            Self::Newest => "launch_date_utc:desc",
            Self::PriceLowToHigh => "current_bid:asc",
            Self::PriceHighToLow => "current_bid:desc",
            Self::RetailHighToLow => "retail_price:desc",
            Self::BiggestDiscount => "discount_percentage:desc",
            Self::MostBids => "total_bids:desc",
        }
    }

    /// Parse user input tolerantly (`ending-soon`, `ending_soon`, `endingsoon`).
    pub fn parse(s: &str) -> Option<Self> {
        let norm = s.trim().to_lowercase().replace(['_', ' '], "-");
        match norm.as_str() {
            "relevance" => Some(Self::Relevance),
            "ending-soon" | "endingsoon" | "closing" => Some(Self::EndingSoon),
            "newest" | "newest-first" | "new" => Some(Self::Newest),
            "price-low-to-high" | "price-asc" | "cheapest" => Some(Self::PriceLowToHigh),
            "price-high-to-low" | "price-desc" => Some(Self::PriceHighToLow),
            "retail-high-to-low" | "retail" => Some(Self::RetailHighToLow),
            "biggest-discount" | "discount" => Some(Self::BiggestDiscount),
            "most-bids" | "bids" => Some(Self::MostBids),
            _ => None,
        }
    }
}

/// A search filter — exactly the facets Mac.Bid's own search supports.
///
/// Every field here is server-side: the query, the facet filters, the order
/// and the pagination all go to the same index the website searches. Nothing
/// in this struct needs a follow-up request per result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchFilter {
    /// Free-text query. `None` (or empty) means browse: the API is asked for
    /// `q=*`, which returns lots without any text match. This is what makes
    /// "show me everything in Televisions" work.
    pub query: Option<String>,
    /// Category names, e.g. `["Televisions"]` (facet `category`).
    pub categories: Vec<String>,
    /// Pickup/auction locations, e.g. `["Robinson", "El Paso"]`
    /// (facet `auction_location`).
    pub locations: Vec<String>,
    /// Two-letter state codes (facet `us_state`).
    pub states: Vec<String>,
    /// Lot conditions (facet `condition`).
    pub conditions: Vec<Condition>,
    /// Minimum current bid.
    pub price_from: Option<f64>,
    /// Maximum current bid.
    pub price_to: Option<f64>,
    /// Minimum retail price.
    pub retail_from: Option<f64>,
    /// Maximum retail price.
    pub retail_to: Option<f64>,
    /// Minimum discount vs retail, in percent (80 = at least 80% off).
    pub discount_min: Option<f64>,
    /// Minimum number of bids already placed.
    pub bids_min: Option<i64>,
    /// Maximum number of bids already placed (0 = untouched lots).
    pub bids_max: Option<i64>,
    /// Only lots closing on/before this day (`YYYY-MM-DD`).
    pub closing_before: Option<String>,
    /// Only lots closing on/after this day (`YYYY-MM-DD`).
    pub closing_after: Option<String>,
    /// Only lots that can be shipped.
    pub shippable: bool,
    /// Only lots transferable to another location.
    pub transferable: bool,
    /// Only "turbo" (short-clock) lots.
    pub turbo: bool,
    /// Only pallet lots. By default pallets are excluded, like the site's
    /// search box does.
    pub pallets_only: bool,
    /// Include pallet lots alongside regular ones (overrides the default
    /// exclusion; ignored when `pallets_only` is set).
    pub include_pallets: bool,
    /// Only loot with free transfer between locations.
    pub free_transfer: bool,
    /// Only featured lots.
    pub featured: bool,
    /// Include lots that already closed / are buffered. Default: live only.
    pub include_closed: bool,
    /// Sort order.
    pub sort: Option<SortOrder>,
    /// 1-based page number.
    pub page: Option<u32>,
    /// Lots per page (max 250).
    pub per_page: Option<u32>,
    /// Facet fields to compute alongside the hits (used by the `facets`
    /// command). Empty means no facets.
    pub facet_by: Vec<String>,
    /// How many facet values to return per field.
    pub max_facet_values: Option<u32>,
}

impl SearchFilter {
    /// True when no text query is set — the caller is browsing a facet.
    pub fn is_browse(&self) -> bool {
        self.query
            .as_deref()
            .map(|q| q.trim().is_empty())
            .unwrap_or(true)
    }

    /// Whether any filter narrows the result set at all.
    pub fn has_any_filter(&self) -> bool {
        !self.categories.is_empty()
            || !self.locations.is_empty()
            || !self.states.is_empty()
            || !self.conditions.is_empty()
            || self.price_from.is_some()
            || self.price_to.is_some()
            || self.retail_from.is_some()
            || self.retail_to.is_some()
            || self.discount_min.is_some()
            || self.bids_min.is_some()
            || self.bids_max.is_some()
            || self.closing_before.is_some()
            || self.closing_after.is_some()
            || self.shippable
            || self.transferable
            || self.turbo
            || self.pallets_only
            || self.free_transfer
            || self.featured
    }

    /// Build the Typesense `filter_by` expression.
    pub fn filter_by(&self) -> String {
        let mut clauses: Vec<String> = Vec::new();

        if !self.include_closed {
            clauses.push("lot_status:=[LIVE]".to_string());
        }
        if self.pallets_only {
            clauses.push("is_pallet:=true".to_string());
        } else if !self.include_pallets {
            clauses.push("is_pallet:=false".to_string());
        }

        push_facet(&mut clauses, "category", &self.categories);
        push_facet(&mut clauses, "auction_location", &self.locations);
        push_facet(&mut clauses, "us_state", &self.states);

        if !self.conditions.is_empty() {
            let any = self
                .conditions
                .iter()
                .map(|c| format!("condition:={}", quote(c.api_value())))
                .collect::<Vec<_>>()
                .join(" || ");
            clauses.push(format!("({any})"));
        }

        if let Some(v) = self.price_from {
            clauses.push(format!("current_bid:>={v}"));
        }
        if let Some(v) = self.price_to {
            clauses.push(format!("current_bid:<={v}"));
        }
        if let Some(v) = self.retail_from {
            clauses.push(format!("retail_price:>={v}"));
        }
        if let Some(v) = self.retail_to {
            clauses.push(format!("retail_price:<={v}"));
        }
        if let Some(v) = self.discount_min {
            clauses.push(format!("discount_percentage:>={v}"));
        }
        if let Some(v) = self.bids_min {
            clauses.push(format!("total_bids:>={v}"));
        }
        if let Some(v) = self.bids_max {
            clauses.push(format!("total_bids:<={v}"));
        }
        if let Some(v) = &self.closing_before {
            clauses.push(format!("expected_close_date:<={}", quote(v)));
        }
        if let Some(v) = &self.closing_after {
            clauses.push(format!("expected_close_date:>={}", quote(v)));
        }

        for (on, field) in [
            (self.shippable, "is_shippable"),
            (self.transferable, "is_transferrable"),
            (self.turbo, "is_turbo"),
            (self.free_transfer, "free_transfer"),
            (self.featured, "is_featured"),
        ] {
            if on {
                clauses.push(format!("{field}:=true"));
            }
        }

        clauses.join(" && ")
    }

    /// Query parameters for the Typesense search endpoint.
    pub fn to_query_pairs(&self) -> Vec<(String, String)> {
        let q = self
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .unwrap_or("*");

        let mut pairs: Vec<(String, String)> = vec![
            ("q".into(), q.to_string()),
            (
                "query_by".into(),
                // `embedding` is deliberately not queried: it drags a vector
                // search into every request for no benefit at this layer.
                "product_name,auction_title,brand,upc,keywords".into(),
            ),
            (
                "exclude_fields".into(),
                "embedding,images,keywords,bid_delta,description,inventory_images".into(),
            ),
        ];

        let filter_by = self.filter_by();
        if !filter_by.is_empty() {
            pairs.push(("filter_by".into(), filter_by));
        }

        let sort = self.sort.unwrap_or_default();
        pairs.push(("sort_by".into(), sort.api_value().to_string()));

        if let Some(page) = self.page {
            pairs.push(("page".into(), page.max(1).to_string()));
        }
        if let Some(per_page) = self.per_page {
            pairs.push(("per_page".into(), per_page.clamp(1, 250).to_string()));
        }
        if !self.facet_by.is_empty() {
            pairs.push(("facet_by".into(), self.facet_by.join(",")));
            pairs.push((
                "max_facet_values".into(),
                self.max_facet_values.unwrap_or(250).to_string(),
            ));
        }
        pairs
    }
}

/// Append `(field:=a || field:=b)` when any value is set.
fn push_facet(clauses: &mut Vec<String>, field: &str, values: &[String]) {
    let values: Vec<&String> = values.iter().filter(|v| !v.trim().is_empty()).collect();
    if values.is_empty() {
        return;
    }
    let any = values
        .iter()
        .map(|v| format!("{field}:={}", quote(v)))
        .collect::<Vec<_>>()
        .join(" || ");
    clauses.push(format!("({any})"));
}

/// Typesense needs backticks around any facet value containing a space or a
/// reserved character. Backticks inside the value are not expressible, so
/// they're stripped rather than allowed to break the expression.
fn quote(value: &str) -> String {
    let clean = value.trim().replace('`', "");
    format!("`{clean}`")
}

// ---------------------------------------------------------------------------
// search documents

/// One lot as returned by the search index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    /// Document key, e.g. `mac_lot_66563445`.
    #[serde(default)]
    pub id: String,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub lot_id: i64,
    #[serde(default)]
    pub lot_number: String,
    #[serde(default)]
    pub product_name: String,
    #[serde(default)]
    pub auction_title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub condition: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub upc: String,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub current_bid: f64,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub retail_price: f64,
    #[serde(default, deserialize_with = "de_opt_f64_lenient")]
    pub instant_win_price: Option<f64>,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub discount: f64,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub discount_percentage: f64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub total_bids: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub unique_bidders: i64,
    #[serde(default)]
    pub auction_id: String,
    #[serde(default)]
    pub auction_number: String,
    #[serde(default)]
    pub auction_location: String,
    #[serde(default)]
    pub us_state: String,
    /// `YYYY-MM-DD` (site-local closing day).
    #[serde(default)]
    pub expected_close_date: String,
    /// Unix seconds when the lot is expected to close.
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub expected_closing_utc: i64,
    /// Unix seconds when the lot launched.
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub launch_date_utc: i64,
    #[serde(default)]
    pub lot_status: String,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_open: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_shippable: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_transferrable: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_turbo: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_pallet: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_featured: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub free_transfer: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub has_video: bool,
    #[serde(default)]
    pub image_url: String,
    #[serde(default, deserialize_with = "de_string_vec")]
    pub transfer_locations: Vec<String>,
}

impl Lot {
    /// Public lot page, built the same way the site builds it.
    pub fn url(&self) -> String {
        if self.auction_id.is_empty() || self.lot_number.is_empty() {
            format!("https://www.mac.bid/lot/{}", self.lot_id)
        } else {
            format!(
                "https://www.mac.bid/auction/{}/lot/{}",
                self.auction_id, self.lot_number
            )
        }
    }

    /// Current bid as a dollar string.
    pub fn bid_str(&self) -> String {
        format!("${:.2}", self.current_bid)
    }
}

/// One facet value and how many docs carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetCount {
    #[serde(default)]
    pub value: String,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub count: i64,
}

/// Facet block keyed by field name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FacetField {
    #[serde(default)]
    pub field_name: String,
    #[serde(default)]
    pub counts: Vec<FacetCount>,
}

/// A Typesense search response (only the fields we use).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    #[serde(default)]
    pub found: i64,
    #[serde(default)]
    pub out_of: i64,
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub facet_counts: Vec<FacetField>,
}

/// One hit: the document plus Typesense's match metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub document: Lot,
}

impl SearchResponse {
    /// Convenience: just the lots, in hit order.
    pub fn lots(self) -> Vec<Lot> {
        self.hits.into_iter().map(|h| h.document).collect()
    }
}

// ---------------------------------------------------------------------------
// lot detail

/// Auction reference embedded in a lot-detail payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuctionRef {
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub id: i64,
    #[serde(default)]
    pub auction_number: String,
    #[serde(default)]
    pub closing_date: Option<String>,
    #[serde(default)]
    pub opening_date: Option<String>,
    #[serde(default)]
    pub pickup_date: Option<String>,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub location_id: i64,
}

/// One image of a lot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LotImage {
    #[serde(default)]
    pub image_url: String,
}

/// Full lot metadata from `/lot/<id>` (title, description, test notes, photos).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LotDetail {
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub id: i64,
    #[serde(default)]
    pub lot_number: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub product_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category_name: String,
    #[serde(default)]
    pub condition_name: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub upc: String,
    #[serde(default, deserialize_with = "de_f64_lenient")]
    pub retail_price: f64,
    #[serde(default, deserialize_with = "de_opt_f64_lenient")]
    pub instant_win_price: Option<f64>,
    /// Current bid. Not part of the lot payload — filled in from the search
    /// index when the lookup succeeds.
    #[serde(default, deserialize_with = "de_opt_f64_lenient")]
    pub current_bid: Option<f64>,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub total_bids: i64,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub unique_bidders: i64,
    #[serde(default, deserialize_with = "de_opt_f64_lenient")]
    pub winning_bid_amount: Option<f64>,
    #[serde(default, deserialize_with = "de_i64_lenient")]
    pub auction_id: i64,
    #[serde(default)]
    pub auction_number: String,
    #[serde(default)]
    pub lot_status: String,
    #[serde(default)]
    pub expected_close_date: Option<String>,
    #[serde(default)]
    pub pickup_deadline: Option<String>,
    #[serde(default)]
    pub auction_closing_date: Option<String>,
    #[serde(default)]
    pub warehouse_location: String,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_shippable: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_transferrable: bool,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub is_tested: bool,
    #[serde(default)]
    pub tested_note: String,
    #[serde(default)]
    pub damaged_note: String,
    #[serde(default)]
    pub image_url: String,
    #[serde(default)]
    pub images: Vec<LotImage>,
    #[serde(default)]
    pub auction: Option<AuctionRef>,
}

impl LotDetail {
    /// Public lot page.
    pub fn url(&self) -> String {
        if self.auction_id == 0 || self.lot_number.is_empty() {
            format!("https://www.mac.bid/lot/{}", self.id)
        } else {
            format!(
                "https://www.mac.bid/auction/{}/lot/{}",
                self.auction_id, self.lot_number
            )
        }
    }

    /// Photo URLs, main image first, de-duplicated.
    pub fn photo_urls(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for url in std::iter::once(&self.image_url).chain(self.images.iter().map(|i| &i.image_url))
        {
            if !url.is_empty() && !out.iter().any(|u| u == url) {
                out.push(url.clone());
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// lenient deserializers

fn de_bool_lenient<'de, D>(d: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().map(|i| i != 0).unwrap_or(false),
        serde_json::Value::String(s) => s == "true" || s == "1",
        _ => false,
    })
}

fn de_i64_lenient<'de, D>(d: D) -> std::result::Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0),
        serde_json::Value::Bool(b) => i64::from(b),
        _ => 0,
    })
}

fn de_f64_lenient<'de, D>(d: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

fn de_opt_f64_lenient<'de, D>(d: D) -> std::result::Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// Accepts `["a", "b"]`, `"a"` or `""` for array-ish fields.
fn de_string_vec<'de, D>(d: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|i| match i {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        serde_json::Value::String(s) if !s.trim().is_empty() => vec![s],
        _ => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browsing() -> SearchFilter {
        SearchFilter::default()
    }

    #[test]
    fn default_filter_excludes_pallets_and_closed_lots() {
        let f = browsing().filter_by();
        assert_eq!(f, "lot_status:=[LIVE] && is_pallet:=false");
    }

    #[test]
    fn category_with_spaces_is_backticked() {
        let f = SearchFilter {
            categories: vec!["Home & Kitchen".into()],
            ..Default::default()
        }
        .filter_by();
        assert!(f.contains("category:=`Home & Kitchen`"), "got {f}");
    }

    #[test]
    fn multiple_categories_become_an_or_group() {
        let f = SearchFilter {
            categories: vec!["Televisions".into(), "Laptops".into()],
            ..Default::default()
        }
        .filter_by();
        assert_eq!(
            f,
            "lot_status:=[LIVE] && is_pallet:=false && (category:=`Televisions` || category:=`Laptops`)"
        );
    }

    #[test]
    fn empty_query_browses_with_star() {
        let pairs = browsing().to_query_pairs();
        let q = pairs.iter().find(|(k, _)| k == "q").map(|(_, v)| v.clone());
        assert_eq!(q.as_deref(), Some("*"));
    }

    #[test]
    fn query_is_trimmed_and_kept() {
        let pairs = SearchFilter {
            query: Some("  lego  ".into()),
            ..Default::default()
        }
        .to_query_pairs();
        let q = pairs.iter().find(|(k, _)| k == "q").map(|(_, v)| v.clone());
        assert_eq!(q.as_deref(), Some("lego"));
    }

    #[test]
    fn numeric_ranges_end_up_in_filter_by() {
        let f = SearchFilter {
            price_from: Some(10.0),
            price_to: Some(99.0),
            discount_min: Some(80.0),
            ..Default::default()
        }
        .filter_by();
        assert!(f.contains("current_bid:>=10"), "got {f}");
        assert!(f.contains("current_bid:<=99"), "got {f}");
        assert!(f.contains("discount_percentage:>=80"), "got {f}");
    }

    #[test]
    fn pallet_and_closed_flags_override_defaults() {
        let f = SearchFilter {
            include_closed: true,
            pallets_only: true,
            ..Default::default()
        }
        .filter_by();
        assert_eq!(f, "is_pallet:=true");
    }

    #[test]
    fn per_page_is_capped_at_the_index_limit() {
        let pairs = SearchFilter {
            per_page: Some(5_000),
            ..Default::default()
        }
        .to_query_pairs();
        let n = pairs
            .iter()
            .find(|(k, _)| k == "per_page")
            .map(|(_, v)| v.clone());
        assert_eq!(n.as_deref(), Some("250"));
    }

    #[test]
    fn lot_deserializes_bool_as_int_and_string_numbers() {
        let doc = serde_json::json!({
            "id": "mac_lot_1",
            "lot_id": 1,
            "current_bid": "12.5",
            "is_shippable": 0,
            "is_transferrable": 1,
            "transfer_locations": "Robinson",
            "product_name": "Thing"
        });
        let lot: Lot = serde_json::from_value(doc).unwrap();
        assert_eq!(lot.current_bid, 12.5);
        assert!(!lot.is_shippable);
        assert!(lot.is_transferrable);
        assert_eq!(lot.transfer_locations, vec!["Robinson"]);
    }

    #[test]
    fn sort_order_parses_user_spellings() {
        assert_eq!(SortOrder::parse("ending-soon"), Some(SortOrder::EndingSoon));
        assert_eq!(SortOrder::parse("Ending Soon"), Some(SortOrder::EndingSoon));
        assert_eq!(
            SortOrder::parse("price_low_to_high"),
            Some(SortOrder::PriceLowToHigh)
        );
        assert_eq!(SortOrder::parse("nonsense"), None);
    }
}
