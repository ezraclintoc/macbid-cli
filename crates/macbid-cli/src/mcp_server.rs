//! `macbid mcp serve` — the Model Context Protocol server.
//!
//! Exposes the same capabilities as the CLI as MCP tools over stdio, reusing
//! the identical search pipeline (Mac.Bid's own facets + local text filters)
//! and the same saved-search/watch files as `macbid save|diff`.
//!
//! The tool schemas advertise only things the index really honors: text
//! query (optional), category/location/state/condition facets, price ranges,
//! closing dates, the pallet/turbo/shipping flags, ordering and pagination.

use crate::{OrderArg, SearchArgs, execute_search, now_secs, run_diff, search_json_value, storage};
use macbid_core::MacBidClient;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// MCP server state. One process per harness session; tools create a
/// `MacBidClient` per call (it is just a reqwest client).
#[derive(Clone)]
pub(crate) struct MacBidMcp {
    tool_router: ToolRouter<Self>,
}

// ---------------------------------------------------------------------------
// tool input types

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SearchParams {
    /// Free-text query. Omit it to browse by filters alone (Mac.Bid's own
    /// index is asked for `q=*`).
    #[serde(default)]
    pub(crate) query: Option<String>,
    /// Category names, e.g. ["Televisions"]. Use list_facets for valid names.
    #[serde(default)]
    pub(crate) category: Vec<String>,
    /// Auction/pickup locations, e.g. ["Robinson", "El Paso"].
    #[serde(default)]
    pub(crate) location: Vec<String>,
    /// Two-letter state codes, e.g. ["PA"].
    #[serde(default)]
    pub(crate) state: Vec<String>,
    /// Conditions: "open-box", "like-new" or "damaged".
    #[serde(default)]
    pub(crate) condition: Vec<String>,
    /// Minimum current bid.
    pub(crate) price_from: Option<f64>,
    /// Maximum current bid.
    pub(crate) price_to: Option<f64>,
    /// Minimum retail price.
    pub(crate) retail_from: Option<f64>,
    /// Maximum retail price.
    pub(crate) retail_to: Option<f64>,
    /// Minimum discount vs retail, in percent (80 = at least 80% off).
    pub(crate) discount_min: Option<f64>,
    /// Minimum bids already placed.
    pub(crate) bids_min: Option<i64>,
    /// Maximum bids already placed (0 = untouched).
    pub(crate) bids_max: Option<i64>,
    /// Only lots closing on/before this day (YYYY-MM-DD).
    pub(crate) closing_before: Option<String>,
    /// Only lots closing on/after this day (YYYY-MM-DD).
    pub(crate) closing_after: Option<String>,
    /// Only lots that can be shipped.
    #[serde(default)]
    pub(crate) shippable: bool,
    /// Only lots transferable to another location.
    #[serde(default)]
    pub(crate) transferable: bool,
    /// Only "turbo" (short-clock) lots.
    #[serde(default)]
    pub(crate) turbo: bool,
    /// Only pallet lots (pallets are excluded by default).
    #[serde(default)]
    pub(crate) pallet: bool,
    /// Include pallet lots alongside regular ones.
    #[serde(default)]
    pub(crate) include_pallets: bool,
    /// Only lots with free transfer between locations.
    #[serde(default)]
    pub(crate) free_transfer: bool,
    /// Only featured lots.
    #[serde(default)]
    pub(crate) featured: bool,
    /// Include lots that already closed (default: live only).
    #[serde(default)]
    pub(crate) include_closed: bool,
    /// Order: relevance | ending-soon | newest | price-low-to-high |
    /// price-high-to-low | retail-high-to-low | biggest-discount | most-bids.
    pub(crate) order: Option<String>,
    /// Fetch N consecutive pages, merged (deeper = slower, more complete).
    pub(crate) pages: Option<u32>,
    /// Results per page (max 250).
    pub(crate) limit: Option<u32>,
    /// Hide lots whose title contains any of these terms (comma separated).
    pub(crate) exclude_text: Option<String>,
    /// Keep only lots whose title contains every query word.
    #[serde(default)]
    pub(crate) titles_only: bool,
    /// Require the exact capitalization of the query words.
    #[serde(default)]
    pub(crate) case_sensitive: bool,
}

impl SearchParams {
    fn to_args(&self) -> SearchArgs {
        SearchArgs {
            query: self.query.clone(),
            category: self.category.clone(),
            location: self.location.clone(),
            state: self.state.clone(),
            condition: self
                .condition
                .iter()
                .filter_map(|c| parse_condition(c))
                .collect(),
            price_from: self.price_from,
            price_to: self.price_to,
            retail_from: self.retail_from,
            retail_to: self.retail_to,
            discount_min: self.discount_min,
            bids_min: self.bids_min,
            bids_max: self.bids_max,
            closing_before: self.closing_before.clone(),
            closing_after: self.closing_after.clone(),
            shippable: self.shippable,
            transferable: self.transferable,
            turbo: self.turbo,
            pallet: self.pallet,
            include_pallets: self.include_pallets,
            free_transfer: self.free_transfer,
            featured: self.featured,
            include_closed: self.include_closed,
            order: self.order.as_deref().and_then(parse_order),
            page: None,
            pages: self.pages,
            limit: self.limit,
            exclude_text: self.exclude_text.clone(),
            titles_only: self.titles_only,
            case_sensitive: self.case_sensitive,
        }
    }
}

/// `open-box | like-new | damaged`, tolerant of spacing/case.
fn parse_condition(s: &str) -> Option<crate::ConditionArg> {
    match s.trim().to_lowercase().replace(['_', ' '], "-").as_str() {
        "open-box" | "openbox" => Some(crate::ConditionArg::OpenBox),
        "like-new" | "likenew" => Some(crate::ConditionArg::LikeNew),
        "damaged" => Some(crate::ConditionArg::Damaged),
        _ => None,
    }
}

/// `relevance | ending-soon | newest | …`, tolerant of separator variants.
fn parse_order(s: &str) -> Option<OrderArg> {
    let norm = s.trim().to_lowercase().replace(['_', ' '], "-");
    match norm.as_str() {
        "relevance" => Some(OrderArg::Relevance),
        "ending-soon" | "endingsoon" | "closing" => Some(OrderArg::EndingSoon),
        "newest" | "newest-first" => Some(OrderArg::Newest),
        "price-low-to-high" | "cheapest" => Some(OrderArg::PriceLowToHigh),
        "price-high-to-low" => Some(OrderArg::PriceHighToLow),
        "retail-high-to-low" | "retail" => Some(OrderArg::RetailHighToLow),
        "biggest-discount" | "discount" => Some(OrderArg::BiggestDiscount),
        "most-bids" | "bids" => Some(OrderArg::MostBids),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct BrowseParams {
    /// Category to browse, e.g. "Televisions" (see list_facets).
    pub(crate) category: String,
    /// Optional extra locations.
    #[serde(default)]
    pub(crate) location: Vec<String>,
    /// Optional minimum current bid.
    pub(crate) price_from: Option<f64>,
    /// Optional maximum current bid.
    pub(crate) price_to: Option<f64>,
    /// Order: ending-soon | newest | price-low-to-high | biggest-discount | …
    pub(crate) order: Option<String>,
    /// Results to return (max 250).
    pub(crate) limit: Option<u32>,
    /// Fetch N consecutive pages, merged.
    pub(crate) pages: Option<u32>,
    /// Hide lots whose title contains any of these terms (comma separated).
    pub(crate) exclude_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct FacetParams {
    /// One of: category, location, condition, state. Omit for all four.
    pub(crate) field: Option<String>,
    /// How many values per field (default 25, 0 = all).
    pub(crate) limit: Option<usize>,
    /// Restrict the counts to these categories.
    #[serde(default)]
    pub(crate) in_category: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct LotId {
    /// Numeric Mac.Bid lot id (the `lot_id` of a search result, or the number
    /// in a /lot/<id> URL).
    pub(crate) id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct WatchName {
    /// Name of the saved search (`watch_save` / `macbid save`).
    pub(crate) name: String,
    /// Override the number of pages fetched for this run.
    pub(crate) pages: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SaveWatchParams {
    /// Name to save the search under (letters, digits, . - _ @).
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) search: SearchParams,
}

// ---------------------------------------------------------------------------
// helpers

fn err(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn snap_json(id: i64, s: &crate::LotSnap) -> serde_json::Value {
    json!({
        "lot_id": id,
        "title": s.title,
        "bid": s.bid,
        "retail": s.retail,
        "closes": s.closes,
        "location": s.location,
        "url": s.url,
    })
}

fn facet_api_field(name: &str) -> Option<&'static str> {
    match name.trim().to_lowercase().as_str() {
        "category" | "categories" => Some("category"),
        "location" | "locations" | "auction_location" => Some("auction_location"),
        "condition" | "conditions" => Some("condition"),
        "state" | "states" | "us_state" => Some("us_state"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// the server

#[tool_router]
impl MacBidMcp {
    pub(crate) fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    fn client(&self) -> Result<MacBidClient, ErrorData> {
        MacBidClient::new().map_err(err)
    }

    #[tool(
        description = "Search Mac.Bid lots. Text is optional: omit `query` to browse by filters alone. Filters are Mac.Bid's own facets: category, auction location, state, condition, price/retail ranges, discount percent, bids, closing dates, pallet/turbo/shippable/transferable flags, plus local exclude_text and titles_only. Returns JSON lots with current bid, retail, discount, closing date, location, bids and url."
    )]
    pub(crate) async fn search_lots(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client()?;
        let run = execute_search(&client, &p.to_args()).await.map_err(err)?;
        let body = search_json_value(&run);
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&body).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Browse a single Mac.Bid category without any search text (e.g. category=\"Televisions\"). Same filters and ordering as search_lots, but the category is required. Use list_facets to discover valid category names and their live lot counts."
    )]
    pub(crate) async fn browse_category(
        &self,
        Parameters(p): Parameters<BrowseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client()?;
        let args = SearchParams {
            query: None,
            category: vec![p.category],
            location: p.location,
            price_from: p.price_from,
            price_to: p.price_to,
            order: p.order,
            limit: p.limit,
            pages: p.pages,
            exclude_text: p.exclude_text,
            ..Default::default()
        }
        .to_args();
        let run = execute_search(&client, &args).await.map_err(err)?;
        let body = search_json_value(&run);
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&body).map_err(err)?,
        )]))
    }

    #[tool(
        description = "List facet values with live lot counts: category, auction location, condition and state (the filters Mac.Bid's own search offers). Use the returned values verbatim in search_lots/browse_category."
    )]
    pub(crate) async fn list_facets(
        &self,
        Parameters(p): Parameters<FacetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client()?;
        let limit = p.limit.unwrap_or(25);
        let base = macbid_core::SearchFilter {
            categories: p.in_category.clone(),
            ..Default::default()
        };

        let fields: Vec<&'static str> = match &p.field {
            Some(f) => vec![facet_api_field(f).ok_or_else(|| {
                ErrorData::invalid_params(
                    format!("unknown facet {f:?}; use category, location, condition or state"),
                    None,
                )
            })?],
            None => vec!["category", "auction_location", "condition", "us_state"],
        };

        let mut out = serde_json::Map::new();
        for field in fields {
            let mut values = client.facet_values_with(field, &base).await.map_err(err)?;
            if limit > 0 {
                values.truncate(limit);
            }
            out.insert(
                field.to_string(),
                json!(
                    values
                        .into_iter()
                        .map(|(value, count)| json!({ "value": value, "count": count }))
                        .collect::<Vec<_>>()
                ),
            );
        }

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&json!(out)).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Full details of one Mac.Bid lot (id from a search result's lot_id or a /lot/<id> URL): description, test/damage notes, condition, retail, buy-now price, current bid, bids, closing date, photos and url."
    )]
    pub(crate) async fn lot_details(
        &self,
        Parameters(p): Parameters<LotId>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.client()?;
        let detail = client.lot(p.id).await.map_err(err)?;
        let mut body = serde_json::to_value(&detail).map_err(err)?;
        if let Some(obj) = body.as_object_mut() {
            obj.insert("url".into(), json!(detail.url()));
            obj.insert("photos".into(), json!(detail.photo_urls()));
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&body).map_err(err)?,
        )]))
    }

    #[tool(
        description = "List every saved search/watch on this machine (same files as `macbid save`/`macbid searches`). Names can be passed to watch_diff."
    )]
    pub(crate) async fn list_watches(&self) -> Result<CallToolResult, ErrorData> {
        let watches: Vec<serde_json::Value> = storage::list_searches()
            .into_iter()
            .map(|name| {
                storage::load_search(&name)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .map(|def| {
                        json!({
                            "name": name,
                            "query": def["args"]["query"],
                            "categories": def["args"]["category"],
                            "locations": def["args"]["location"],
                        })
                    })
                    .unwrap_or_else(|| json!({ "name": name, "unreadable": true }))
            })
            .collect();
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&json!({ "watches": watches })).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Run a saved search/watch and report what changed since the last run: newly listed lots ('added') and lots that left the results ('gone' — closed, sold to someone else, or pushed out of the result window). The first run records a baseline. State lives in macbid-cli's config dir; CLI `macbid diff` uses the same files."
    )]
    pub(crate) async fn watch_diff(
        &self,
        Parameters(p): Parameters<WatchName>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = storage::sanitize(&p.name).map_err(err)?;
        let def_text = storage::load_search(&name).map_err(err)?;
        #[derive(Deserialize)]
        struct Saved {
            args: SearchArgs,
        }
        let saved: Saved = serde_json::from_str(&def_text).map_err(err)?;
        let mut args = saved.args;
        if let Some(pages) = p.pages {
            args.pages = Some(pages);
            args.page = None;
        }
        let client = self.client()?;
        let out = run_diff(&name, &client, &args).await.map_err(err)?;
        let body = match out.baseline {
            Some(lots) => json!({ "name": name, "baseline_recorded": true, "lots": lots }),
            None => json!({
                "name": name,
                "added": out.added.iter().map(|(id, s)| snap_json(*id, s)).collect::<Vec<_>>(),
                "gone": out.gone.iter().map(|(id, s)| snap_json(*id, s)).collect::<Vec<_>>(),
                "unchanged": out.unchanged,
            }),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&body).map_err(err)?,
        )]))
    }

    #[tool(
        description = "Create/overwrite a saved search (watch) so it can be polled later with watch_diff. Same files `macbid save` writes; accepts the same filters as search_lots."
    )]
    pub(crate) async fn watch_save(
        &self,
        Parameters(p): Parameters<SaveWatchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = storage::sanitize(&p.name).map_err(err)?;
        let def = json!({
            "name": name,
            "created_at": now_secs(),
            "args": p.search.to_args(),
        });
        let existed =
            storage::save_search(&name, &serde_json::to_string_pretty(&def).map_err(err)?)
                .map_err(err)?;
        let note = if existed {
            format!("saved search `{name}` overwritten (previous watch state kept)")
        } else {
            format!(
                "saved search `{name}` created — poll it with watch_diff; the first run records a baseline"
            )
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(note)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MacBidMcp {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("macbid", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Read-only Mac.Bid auction search: lots with current bid, retail price, discount, closing time and pickup location. Filters are Mac.Bid's own facets — browse a category with no text query, narrow by location/state/condition/price, and diff saved watches to catch new or disappeared lots. Use list_facets first to learn valid category and location names."
                .to_string(),
        );
        info
    }
}

pub(crate) async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let service = MacBidMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_strings_parse_like_the_cli() {
        assert!(matches!(
            parse_order("ending-soon"),
            Some(OrderArg::EndingSoon)
        ));
        assert!(matches!(
            parse_order("Biggest Discount"),
            Some(OrderArg::BiggestDiscount)
        ));
        assert!(parse_order("sideways").is_none());
    }

    #[test]
    fn conditions_parse_leniently() {
        assert!(matches!(
            parse_condition("like new"),
            Some(crate::ConditionArg::LikeNew)
        ));
        assert!(matches!(
            parse_condition("open_box"),
            Some(crate::ConditionArg::OpenBox)
        ));
        assert!(parse_condition("mint").is_none());
    }

    #[test]
    fn facet_names_are_aliased_to_index_fields() {
        assert_eq!(facet_api_field("categories"), Some("category"));
        assert_eq!(facet_api_field("state"), Some("us_state"));
        assert_eq!(facet_api_field("brand"), None);
    }

    #[test]
    fn browse_tool_args_carry_the_category_and_no_query() {
        let params = BrowseParams {
            category: "Televisions".into(),
            location: vec![],
            price_from: None,
            price_to: Some(300.0),
            order: Some("price-low-to-high".into()),
            limit: Some(10),
            pages: None,
            exclude_text: None,
        };
        let args = SearchParams {
            query: None,
            category: vec![params.category],
            price_to: params.price_to,
            order: params.order,
            limit: params.limit,
            ..Default::default()
        }
        .to_args();
        assert!(args.query.is_none());
        assert_eq!(args.category, vec!["Televisions"]);
        assert_eq!(args.price_to, Some(300.0));
        assert_eq!(args.limit, Some(10));
    }
}
