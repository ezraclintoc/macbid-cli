//! `macbid` — search, filter and watch Mac.Bid auctions from your terminal.
//!
//! Every filter maps to a facet Mac.Bid's own search exposes (category,
//! auction location, state, condition, price, closing date, pallet/turbo/
//! shipping flags), so results match what the site would show — just
//! scriptable, and watchable via `save`/`diff` or the MCP server.

pub(crate) mod storage;

mod mcp_server;

use clap::{Args, Parser, Subcommand, ValueEnum};
use comfy_table::presets::ASCII_MARKDOWN;
use comfy_table::{Cell, Color, Table};
use macbid_core::{
    Condition, Lot, LotDetail, MacBidClient, SearchFilter, SortOrder,
    local_filters::{CaseMode, LocalFilter},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "macbid",
    version,
    about = "Search, filter and watch Mac.Bid auctions from your terminal.",
    after_help = "Every filter is one of Mac.Bid's own facets. Omit the QUERY to browse a category."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Output raw JSON instead of a formatted table.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Search lots. The QUERY is optional: leave it out and filter by
    /// category/location/price only (that's what `browse` does).
    Search {
        #[command(flatten)]
        args: SearchArgs,
    },

    /// Browse one category without typing any query text.
    Browse {
        /// Category to browse, e.g. `Televisions` (see `macbid facets`).
        #[arg(long, short = 'c', value_name = "NAME")]
        category: String,

        #[command(flatten)]
        args: BrowseArgs,
    },

    /// List the values of one facet — categories, auction locations,
    /// conditions or states — with live lot counts (alias: `categories`).
    #[command(alias = "categories")]
    Facets {
        /// Which facet to list. Without it, the top values of all four.
        #[arg(long, value_enum, value_name = "FIELD")]
        field: Option<FacetFieldArg>,

        /// How many values to show per field (default 20, 0 = all).
        #[arg(long, short = 'n', value_name = "N", default_value_t = 20)]
        limit: usize,

        /// Count values within these categories instead of all live lots.
        #[arg(long = "in-category", value_name = "NAME")]
        in_category: Vec<String>,
    },

    /// Show full details for one lot (description, test notes, photos).
    Lot {
        /// Numeric lot id (the number in `/lot/<id>`, or the `lot_id` field
        /// of a search result).
        id: i64,
    },

    /// Save a search so you can re-run and diff it later (`macbid diff NAME`).
    Save {
        /// Name to save the search under (letters, digits, . - _ @).
        name: String,

        #[command(flatten)]
        args: SearchArgs,
    },

    /// Re-run a saved search and show what changed since the last run:
    /// newly listed lots (added) and lots that left the results
    /// (closed/gone). The first run just records a baseline.
    Diff {
        /// Name the search was saved under.
        name: String,

        /// Override the number of pages fetched (deeper = more reliable
        /// change detection, slower).
        #[arg(long, value_name = "N")]
        pages: Option<u32>,

        /// Also write the diff to this CSV file
        /// (columns: status,lot_id,title,bid,closes,location,url).
        #[arg(long, value_name = "FILE")]
        export: Option<PathBuf>,
    },

    /// List saved searches.
    Searches,

    /// Delete a saved search (and any recorded watch state).
    Forget {
        /// Name the search was saved under.
        name: String,
    },

    /// Run the MCP (Model Context Protocol) server over stdio.
    Mcp {
        #[command(subcommand)]
        mcp_command: McpCommand,
    },
}

/// Subcommands of `macbid mcp`.
#[derive(Subcommand, Debug)]
enum McpCommand {
    /// Serve the Mac.Bid MCP tools over stdio (long-running; launched by the
    /// agent harness, not by hand).
    Serve,
}

/// Flags shared by `browse` (which supplies the category itself).
#[derive(Args, Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct BrowseArgs {
    /// Also filter by auction/pickup location. Repeatable.
    #[arg(long, short = 'l', value_name = "NAME", value_delimiter = ',')]
    location: Vec<String>,

    /// Filter by two-letter state code. Repeatable.
    #[arg(long, value_name = "STATE", value_delimiter = ',')]
    state: Vec<String>,

    /// Filter by condition. Repeatable.
    #[arg(long, value_enum, value_name = "COND")]
    condition: Vec<ConditionArg>,

    /// Minimum current bid.
    #[arg(long, value_name = "PRICE")]
    price_from: Option<f64>,

    /// Maximum current bid.
    #[arg(long, value_name = "PRICE")]
    price_to: Option<f64>,

    /// Minimum discount vs retail, in percent (80 = at least 80% off).
    #[arg(long, value_name = "PERCENT")]
    discount_min: Option<f64>,

    /// Result order.
    #[arg(long, value_enum, value_name = "ORDER")]
    order: Option<OrderArg>,

    /// Results per page (max 250).
    #[arg(long, short = 'n', value_name = "N")]
    limit: Option<u32>,

    /// Fetch N consecutive pages (merged).
    #[arg(long, value_name = "N")]
    pages: Option<u32>,

    /// Hide lots whose title contains any of these terms (comma separated).
    #[arg(long, value_name = "WORDS")]
    exclude_text: Option<String>,
}

impl BrowseArgs {
    fn into_search_args(self, category: String) -> SearchArgs {
        SearchArgs {
            query: None,
            category: vec![category],
            location: self.location,
            state: self.state,
            condition: self.condition,
            price_from: self.price_from,
            price_to: self.price_to,
            discount_min: self.discount_min,
            order: self.order,
            limit: self.limit,
            pages: self.pages,
            exclude_text: self.exclude_text,
            ..Default::default()
        }
    }
}

/// Everything `macbid search` accepts; shared by `macbid save`/`diff` and the
/// MCP tools — saved searches persist (and reload) this exact set of flags.
#[derive(Args, Clone, Debug, Serialize, Deserialize, Default)]
// `serde(default)` keeps saved searches written by older versions loadable
// when new filters are added later.
#[serde(default)]
pub(crate) struct SearchArgs {
    /// Free-text query. Leave it out to browse by filter alone.
    query: Option<String>,

    /// Category name, e.g. `Televisions`. Repeatable (comma separated).
    #[arg(long, short = 'c', value_name = "NAME", value_delimiter = ',')]
    category: Vec<String>,

    /// Auction/pickup location, e.g. `Robinson`. Repeatable.
    #[arg(long, short = 'l', value_name = "NAME", value_delimiter = ',')]
    location: Vec<String>,

    /// Two-letter state code, e.g. `PA`. Repeatable.
    #[arg(long, value_name = "STATE", value_delimiter = ',')]
    state: Vec<String>,

    /// Lot condition. Repeatable.
    #[arg(long, value_enum, value_name = "COND")]
    condition: Vec<ConditionArg>,

    /// Minimum current bid.
    #[arg(long, value_name = "PRICE")]
    price_from: Option<f64>,

    /// Maximum current bid.
    #[arg(long, value_name = "PRICE")]
    price_to: Option<f64>,

    /// Minimum retail price.
    #[arg(long, value_name = "PRICE")]
    retail_from: Option<f64>,

    /// Maximum retail price.
    #[arg(long, value_name = "PRICE")]
    retail_to: Option<f64>,

    /// Minimum discount vs retail, in percent (80 = at least 80% off).
    #[arg(long, value_name = "PERCENT")]
    discount_min: Option<f64>,

    /// Minimum number of bids already placed.
    #[arg(long, value_name = "N")]
    bids_min: Option<i64>,

    /// Maximum number of bids already placed (0 = untouched lots).
    #[arg(long, value_name = "N")]
    bids_max: Option<i64>,

    /// Only lots closing on/before this day (`YYYY-MM-DD`).
    #[arg(long, value_name = "DATE")]
    closing_before: Option<String>,

    /// Only lots closing on/after this day (`YYYY-MM-DD`).
    #[arg(long, value_name = "DATE")]
    closing_after: Option<String>,

    /// Only lots that can be shipped.
    #[arg(long)]
    shippable: bool,

    /// Only lots transferable to another location.
    #[arg(long)]
    transferable: bool,

    /// Only "turbo" (short-clock) lots.
    #[arg(long)]
    turbo: bool,

    /// Only pallet lots (pallets are excluded by default, like the site).
    #[arg(long, conflicts_with = "include_pallets")]
    pallet: bool,

    /// Include pallet lots too, alongside regular ones.
    #[arg(long)]
    include_pallets: bool,

    /// Only lots with free transfer between locations.
    #[arg(long)]
    free_transfer: bool,

    /// Only featured lots.
    #[arg(long)]
    featured: bool,

    /// Include lots that already closed (default: live lots only).
    #[arg(long)]
    include_closed: bool,

    /// Result order.
    #[arg(long, value_enum, value_name = "ORDER")]
    order: Option<OrderArg>,

    /// Page number (1-based).
    #[arg(long, short = 'p', value_name = "N")]
    page: Option<u32>,

    /// Fetch N consecutive pages (merged, de-duplicated).
    #[arg(long, value_name = "N", conflicts_with = "page")]
    pages: Option<u32>,

    /// Results per page (max 250).
    #[arg(long, short = 'n', value_name = "N")]
    limit: Option<u32>,

    /// Hide lots whose title contains any of these terms (comma separated;
    /// case-insensitive unless --case-sensitive).
    #[arg(long, value_name = "WORDS")]
    exclude_text: Option<String>,

    /// Ignore matches found only in the brand/UPC/keywords: keep only lots
    /// whose title contains every query word.
    #[arg(long)]
    titles_only: bool,

    /// Enforce exact capitalization locally: title matches must use the case
    /// you typed.
    #[arg(long)]
    case_sensitive: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, Serialize, Deserialize)]
enum ConditionArg {
    OpenBox,
    LikeNew,
    Damaged,
}

impl From<ConditionArg> for Condition {
    fn from(a: ConditionArg) -> Self {
        match a {
            ConditionArg::OpenBox => Self::OpenBox,
            ConditionArg::LikeNew => Self::LikeNew,
            ConditionArg::Damaged => Self::Damaged,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) enum OrderArg {
    Relevance,
    EndingSoon,
    Newest,
    PriceLowToHigh,
    PriceHighToLow,
    RetailHighToLow,
    BiggestDiscount,
    MostBids,
}

impl From<OrderArg> for SortOrder {
    fn from(a: OrderArg) -> Self {
        match a {
            OrderArg::Relevance => Self::Relevance,
            OrderArg::EndingSoon => Self::EndingSoon,
            OrderArg::Newest => Self::Newest,
            OrderArg::PriceLowToHigh => Self::PriceLowToHigh,
            OrderArg::PriceHighToLow => Self::PriceHighToLow,
            OrderArg::RetailHighToLow => Self::RetailHighToLow,
            OrderArg::BiggestDiscount => Self::BiggestDiscount,
            OrderArg::MostBids => Self::MostBids,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum FacetFieldArg {
    Category,
    Location,
    Condition,
    State,
}

impl FacetFieldArg {
    fn api_field(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Location => "auction_location",
            Self::Condition => "condition",
            Self::State => "us_state",
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// diff state

/// Snapshot of a lot as stored in watch state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LotSnap {
    title: String,
    #[serde(default)]
    bid: String,
    #[serde(default)]
    retail: String,
    #[serde(default)]
    closes: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    url: String,
}

/// A previously-seen lot that disappeared from the results.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoneRec {
    lot: LotSnap,
    /// Unix seconds when the lot was last seen missing.
    #[allow(dead_code)] // persisted for future "how long ago" reporting
    gone_at: u64,
}

/// Watch state for `macbid diff`: what was seen last run, plus an
/// accumulating ledger of everything that left the results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WatchState {
    active: BTreeMap<i64, LotSnap>,
    gone: BTreeMap<i64, GoneRec>,
    #[serde(default)]
    last_run: u64,
}

pub(crate) fn snap(lot: &Lot) -> LotSnap {
    LotSnap {
        title: lot.title().to_string(),
        bid: format!("${:.2}", lot.current_bid),
        retail: format!("${:.2}", lot.retail_price),
        closes: closes_label(lot),
        location: lot.auction_location.clone(),
        url: lot.url(),
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Search { args } => {
            let client = MacBidClient::new()?;
            let run = execute_search(&client, &args).await?;
            if cli.json {
                print_search_json(&run);
            } else {
                print_search_table(&run);
            }
        }

        Command::Browse { category, args } => {
            let client = MacBidClient::new()?;
            let run = execute_search(&client, &args.into_search_args(category)).await?;
            if cli.json {
                print_search_json(&run);
            } else {
                print_search_table(&run);
            }
        }

        Command::Facets {
            field,
            limit,
            in_category,
        } => {
            let client = MacBidClient::new()?;
            let filter = SearchFilter {
                categories: in_category,
                ..Default::default()
            };
            let fields: Vec<&'static str> = match field {
                Some(f) => vec![f.api_field()],
                None => vec!["category", "auction_location", "condition", "us_state"],
            };
            if cli.json {
                let mut out = serde_json::Map::new();
                for name in fields {
                    let values = client.facet_values_with(name, &filter).await?;
                    let values: Vec<serde_json::Value> = values
                        .into_iter()
                        .map(|(value, count)| json!({ "value": value, "count": count }))
                        .collect();
                    out.insert(name.to_string(), json!(values));
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for name in fields {
                    let mut values = client.facet_values_with(name, &filter).await?;
                    if limit > 0 {
                        values.truncate(limit);
                    }
                    println!("\n{}", name.replace('_', " ").to_uppercase());
                    if values.is_empty() {
                        println!("  (none)");
                    }
                    for (value, count) in values {
                        println!("  {value:<28} {count:>7}");
                    }
                }
            }
        }

        Command::Lot { id } => {
            let client = MacBidClient::new()?;
            let detail = client.lot(id).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&detail_json(&detail))?);
            } else {
                print_lot_detail(&detail);
            }
        }

        Command::Save { name, args } => {
            let name = storage::sanitize(&name)?;
            let def = json!({
                "name": name,
                "created_at": now_secs(),
                "args": args,
            });
            let existed = storage::save_search(&name, &serde_json::to_string_pretty(&def)?)?;
            if existed {
                eprintln!("(overwriting previously saved search `{name}`)");
            }
            println!("saved search `{name}` — run it and diff with: macbid diff {name}");
        }

        Command::Searches => {
            let names = storage::list_searches();
            if names.is_empty() {
                println!(
                    "No saved searches yet. Create one with: macbid save NAME [QUERY] [flags]"
                );
                return Ok(());
            }
            for name in names {
                let summary = storage::load_search(&name)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .map(|v| {
                        let q = v["args"]["query"]
                            .as_str()
                            .unwrap_or("(browse)")
                            .to_string();
                        let cats = v["args"]["category"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "-".to_string());
                        format!("query={q:?} categories={cats}")
                    })
                    .unwrap_or_else(|| "(unreadable definition)".to_string());
                println!("{name:<24} {summary}");
            }
        }

        Command::Forget { name } => {
            let name = storage::sanitize(&name)?;
            let removed_search = storage::delete_search(&name);
            let removed_state = storage::delete_state(&name);
            if removed_search || removed_state {
                println!("forgot `{name}`");
            } else {
                println!("no saved search named `{name}`");
            }
        }

        Command::Diff {
            name,
            pages,
            export,
        } => {
            let name = storage::sanitize(&name)?;
            let def_text = storage::load_search(&name)?;
            #[derive(Deserialize)]
            struct SavedSearch {
                args: SearchArgs,
            }
            let def: SavedSearch = serde_json::from_str(&def_text)
                .map_err(|e| format!("saved search `{name}` is unreadable ({e}); re-save it"))?;
            let mut args = def.args;
            if let Some(p) = pages {
                args.pages = Some(p);
                args.page = None;
            }
            let client = MacBidClient::new()?;
            diff(cli.json, &name, &client, &args, export).await?;
        }

        Command::Mcp { mcp_command } => match mcp_command {
            McpCommand::Serve => {
                mcp_server::serve().await.map_err(|e| e.to_string())?;
            }
        },
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// search pipeline (shared by `search`, `browse`, `diff` and MCP)

/// One executed search.
pub(crate) struct Run {
    pub(crate) lots: Vec<Lot>,
    pub(crate) total_found: i64,
    pub(crate) pages_fetched: u32,
    /// Lots removed by the local (client-side) text filters.
    pub(crate) dropped: usize,
    /// The `filter_by` expression that was sent (handy for debugging).
    pub(crate) filter_by: String,
    pub(crate) page: u32,
    pub(crate) per_page: u32,
}

/// Turn parsed flags into a server-side [`SearchFilter`].
pub(crate) fn build_filter(args: &SearchArgs) -> Result<SearchFilter, String> {
    for (flag, date) in [
        ("--closing-before", &args.closing_before),
        ("--closing-after", &args.closing_after),
    ] {
        if let Some(d) = date
            && !is_iso_date(d)
        {
            return Err(format!("{flag} wants YYYY-MM-DD, got {d:?}"));
        }
    }
    if let (Some(from), Some(to)) = (args.price_from, args.price_to)
        && from > to
    {
        return Err(format!("--price-from {from} is above --price-to {to}"));
    }

    Ok(SearchFilter {
        query: args.query.clone(),
        categories: args.category.clone(),
        locations: args.location.clone(),
        states: args.state.iter().map(|s| s.to_uppercase()).collect(),
        conditions: args.condition.iter().map(|&c| Condition::from(c)).collect(),
        price_from: args.price_from,
        price_to: args.price_to,
        retail_from: args.retail_from,
        retail_to: args.retail_to,
        discount_min: args.discount_min,
        bids_min: args.bids_min,
        bids_max: args.bids_max,
        closing_before: args.closing_before.clone(),
        closing_after: args.closing_after.clone(),
        shippable: args.shippable,
        transferable: args.transferable,
        turbo: args.turbo,
        pallets_only: args.pallet,
        include_pallets: args.include_pallets,
        free_transfer: args.free_transfer,
        featured: args.featured,
        include_closed: args.include_closed,
        sort: args.order.map(Into::into),
        page: args.page,
        per_page: args.limit,
        facet_by: Vec::new(),
        max_facet_values: None,
    })
}

/// `YYYY-MM-DD` sanity check (the index compares these lexicographically).
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit)
}

fn local_filter(args: &SearchArgs) -> LocalFilter {
    // Keep only lots whose title contains every query word: that's what
    // --titles-only means here, and --case-sensitive implies it too (the
    // server has no notion of capitalization).
    let require_words = if args.case_sensitive || args.titles_only {
        args.query
            .as_ref()
            .map(|q| q.split_whitespace().map(|w| w.to_string()).collect())
            .unwrap_or_default()
    } else {
        Default::default()
    };
    LocalFilter {
        exclude_text: split_comma(&args.exclude_text),
        require_words,
        case: if args.case_sensitive {
            CaseMode::Exact
        } else {
            CaseMode::IgnoreCase
        },
    }
}

fn split_comma(opt: &Option<String>) -> Vec<String> {
    opt.as_ref()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch + filter: one server search (single or multi-page), then the local
/// text filters.
pub(crate) async fn execute_search(
    client: &MacBidClient,
    args: &SearchArgs,
) -> Result<Run, Box<dyn std::error::Error>> {
    let filter = build_filter(args)?;
    let filter_by = filter.filter_by();

    let (mut lots, total_found, pages_fetched) = if let Some(pages) = args.pages {
        let merged = client.search_pages(&filter, pages).await?;
        (merged.lots, merged.total_found, merged.pages_fetched)
    } else {
        let resp = client.search(&filter).await?;
        let found = resp.found;
        (resp.lots(), found, 1)
    };
    if pages_fetched > 1 {
        eprintln!("→ fetched {pages_fetched} page(s)");
    }

    let (kept, dropped) = local_filter(args).apply(std::mem::take(&mut lots));
    lots = kept;

    Ok(Run {
        lots,
        total_found,
        pages_fetched,
        dropped,
        filter_by,
        page: args.page.unwrap_or(1),
        per_page: args.limit.unwrap_or(10).clamp(1, 250),
    })
}

// ---------------------------------------------------------------------------
// diff

/// One watch-diff run's delta (shared by `macbid diff` and the MCP
/// `watch_diff` tool; printing and CSV export are the caller's job).
pub(crate) struct DiffOut {
    pub(crate) added: Vec<(i64, LotSnap)>,
    pub(crate) gone: Vec<(i64, LotSnap)>,
    pub(crate) unchanged: usize,
    /// Set on the first run: a baseline was recorded with this many lots.
    pub(crate) baseline: Option<usize>,
    pub(crate) pages_fetched: u32,
}

/// Execute the saved search `args`, compare against the watch state named
/// `name`, update the state, and return the delta.
pub(crate) async fn run_diff(
    name: &str,
    client: &MacBidClient,
    args: &SearchArgs,
) -> Result<DiffOut, Box<dyn std::error::Error>> {
    let run = execute_search(client, args).await?;
    let current: Vec<(i64, LotSnap)> = run.lots.iter().map(|lot| (lot.lot_id, snap(lot))).collect();

    let now = now_secs();
    let prev: Option<WatchState> =
        storage::load_state(name).and_then(|text| serde_json::from_str(&text).ok());

    // First run: record a baseline instead of calling all N lots "added".
    let Some(mut state) = prev else {
        let new_state = WatchState {
            active: current.iter().cloned().collect(),
            gone: BTreeMap::new(),
            last_run: now,
        };
        storage::save_state(name, &serde_json::to_string_pretty(&new_state)?)?;
        return Ok(DiffOut {
            added: Vec::new(),
            gone: Vec::new(),
            unchanged: 0,
            baseline: Some(current.len()),
            pages_fetched: run.pages_fetched,
        });
    };

    let current_ids: HashSet<i64> = current.iter().map(|(id, _)| *id).collect();

    let mut added: Vec<(i64, LotSnap)> = Vec::new();
    for (id, lot) in &current {
        if !state.active.contains_key(id) {
            if state.gone.remove(id).is_some() {
                // The lot is back in the results (relisted, or it was just
                // pushed out of the result window last time).
            }
            added.push((*id, lot.clone()));
        }
    }

    let mut gone: Vec<(i64, LotSnap)> = Vec::new();
    for (id, lot) in state.active.clone() {
        if !current_ids.contains(&id) {
            state.gone.insert(
                id,
                GoneRec {
                    lot: lot.clone(),
                    gone_at: now,
                },
            );
            gone.push((id, lot));
        }
    }

    // Refresh the active snapshot (bids/titles change between runs).
    state.active = current.iter().cloned().collect();
    state.last_run = now;
    storage::save_state(name, &serde_json::to_string_pretty(&state)?)?;

    let unchanged = current.len().saturating_sub(added.len());

    Ok(DiffOut {
        added,
        gone,
        unchanged,
        baseline: None,
        pages_fetched: run.pages_fetched,
    })
}

async fn diff(
    json: bool,
    name: &str,
    client: &MacBidClient,
    args: &SearchArgs,
    export: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let out = run_diff(name, client, args).await?;

    if let Some(baseline) = out.baseline {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "baseline_recorded": true,
                    "lots": baseline,
                }))?
            );
        } else {
            println!(
                "Baseline recorded: {baseline} lots seen. Run `macbid diff {name}` again to see added and gone lots."
            );
        }
    } else if json {
        let entries = |list: &[(i64, LotSnap)]| {
            list.iter()
                .map(|(id, s)| {
                    json!({
                        "lot_id": id,
                        "title": s.title,
                        "bid": s.bid,
                        "retail": s.retail,
                        "closes": s.closes,
                        "location": s.location,
                        "url": s.url,
                    })
                })
                .collect::<Vec<_>>()
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "added": entries(&out.added),
                "gone": entries(&out.gone),
                "unchanged": out.unchanged,
            }))?
        );
    } else if out.added.is_empty() && out.gone.is_empty() {
        println!(
            "No changes since the last run ({} lots unchanged).",
            out.unchanged
        );
    } else {
        if !out.added.is_empty() {
            println!("+ added ({}):", out.added.len());
            for (id, lot) in &out.added {
                println!(
                    "  [{id}] {} — {} — {} — {}",
                    truncate(&lot.title, 58),
                    lot.bid,
                    lot.location,
                    lot.closes
                );
            }
        }
        if !out.gone.is_empty() {
            println!("- gone ({}):", out.gone.len());
            for (id, lot) in &out.gone {
                println!(
                    "  [{id}] {} — {} — {} — {}",
                    truncate(&lot.title, 58),
                    lot.bid,
                    lot.location,
                    lot.closes
                );
            }
        }
        println!("{} lots unchanged", out.unchanged);
    }

    if let Some(path) = export {
        export_csv(&path, &out.added, &out.gone)?;
        eprintln!("→ wrote diff to {}", path.display());
    }
    if out.pages_fetched > 1 {
        eprintln!("(checked {} page(s))", out.pages_fetched);
    }
    Ok(())
}

fn export_csv(
    path: &PathBuf,
    added: &[(i64, LotSnap)],
    gone: &[(i64, LotSnap)],
) -> Result<(), String> {
    let mut out = String::from("status,lot_id,title,bid,retail,closes,location,url\n");
    let mut rows = 0usize;
    let push_row = |out: &mut String, status: &str, id: i64, lot: &LotSnap| {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            status,
            id,
            csv_cell(&lot.title),
            csv_cell(&lot.bid),
            csv_cell(&lot.retail),
            csv_cell(&lot.closes),
            csv_cell(&lot.location),
            csv_cell(&lot.url),
        ));
    };
    for (id, lot) in added {
        push_row(&mut out, "added", *id, lot);
        rows += 1;
    }
    for (id, lot) in gone {
        push_row(&mut out, "gone", *id, lot);
        rows += 1;
    }
    std::fs::write(path, out).map_err(|e| format!("writing {}: {e}", path.display()))?;
    eprintln!("wrote {rows} diff row(s) to {}", path.display());
    Ok(())
}

fn csv_cell(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------
// output helpers

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

/// `10-29` from `2026-10-29`, so the table stays narrow.
fn closes_label(lot: &Lot) -> String {
    let d = &lot.expected_close_date;
    if d.len() == 10 {
        d[5..].to_string()
    } else if d.is_empty() {
        "—".into()
    } else {
        d.clone()
    }
}

fn print_search_table(run: &Run) {
    if run.lots.is_empty() {
        println!("No lots matched your filters.");
        return;
    }

    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(vec![
        "lot", "title", "bid", "retail", "off", "closes", "location", "bids",
    ]);

    for lot in &run.lots {
        let off = if lot.discount_percentage > 0.0 {
            format!("{:.0}%", lot.discount_percentage)
        } else {
            "—".into()
        };
        table.add_row(vec![
            Cell::new(lot.lot_id),
            Cell::new(truncate(lot.title(), 52)),
            Cell::new(format!("{:.2}", lot.current_bid)).fg(Color::Green),
            Cell::new(format!("{:.2}", lot.retail_price)),
            Cell::new(off),
            Cell::new(closes_label(lot)),
            Cell::new(truncate(&lot.auction_location, 16)),
            Cell::new(lot.total_bids),
        ]);
    }

    println!("{table}");

    let shown = run.lots.len();
    if run.pages_fetched > 1 {
        println!(
            "{shown} lots shown ({pages} pages merged, of {found} matching)",
            pages = run.pages_fetched,
            found = run.total_found
        );
    } else {
        let page = run.page.max(1);
        let pages = (run.total_found as f64 / run.per_page as f64).ceil() as i64;
        println!(
            "{shown} lots shown (page {page}/{pages} of {found} matching)",
            found = run.total_found
        );
    }
    if run.dropped > 0 {
        println!("(dropped {} lots by local text filters)", run.dropped);
    }
}

/// Machine-readable view of one search run (shared by `macbid search --json`
/// and the MCP `search_lots` tool).
pub(crate) fn search_json_value(run: &Run) -> serde_json::Value {
    json!({
        "lots": run.lots.iter().map(lot_json).collect::<Vec<_>>(),
        "found": run.total_found,
        "shown": run.lots.len(),
        "page": run.page,
        "per_page": run.per_page,
        "pages_fetched": run.pages_fetched,
        "filter_by": run.filter_by,
        "dropped_by_local_filters": run.dropped,
    })
}

fn lot_json(lot: &Lot) -> serde_json::Value {
    let mut v = serde_json::to_value(lot).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("url".into(), json!(lot.url()));
    }
    v
}

fn detail_json(detail: &LotDetail) -> serde_json::Value {
    let mut v = serde_json::to_value(detail).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("url".into(), json!(detail.url()));
        obj.insert("photos".into(), json!(detail.photo_urls()));
    }
    v
}

fn print_search_json(run: &Run) {
    println!(
        "{}",
        serde_json::to_string_pretty(&search_json_value(run)).unwrap()
    );
}

fn print_lot_detail(d: &LotDetail) {
    println!("lot:       {} ({})", d.lot_number, d.id);
    println!(
        "title:     {}",
        if d.title.is_empty() {
            &d.product_name
        } else {
            &d.title
        }
    );
    if !d.category_name.is_empty() {
        println!("category:  {}", d.category_name);
    }
    if !d.condition_name.is_empty() {
        println!("condition: {}", d.condition_name);
    }
    if let Some(bid) = d.current_bid {
        println!("bid:       ${bid:.2}");
    }
    if d.retail_price > 0.0 {
        println!("retail:    ${:.2}", d.retail_price);
    }
    if let Some(iw) = d.instant_win_price {
        println!("buy now:   ${iw:.2}");
    }
    println!("bids:      {} ({} bidders)", d.total_bids, d.unique_bidders);
    let closes = d
        .expected_close_date
        .as_deref()
        .map(|s| truncate(s, 16))
        .unwrap_or_else(|| "—".into());
    println!("closes:    {closes}");
    if let Some(a) = &d.auction {
        println!(
            "auction:   {} ({})",
            d.auction_number,
            a.pickup_date.clone().unwrap_or_default()
        );
    }
    println!(
        "flags:     {}{}{}",
        if d.is_shippable { "shippable " } else { "" },
        if d.is_transferrable {
            "transferable "
        } else {
            ""
        },
        if d.is_tested { "tested" } else { "" }
    );
    if !d.tested_note.is_empty() {
        println!("tested:    {}", truncate(&d.tested_note, 200));
    }
    if !d.damaged_note.is_empty() {
        println!("damage:    {}", truncate(&d.damaged_note, 200));
    }
    if !d.description.is_empty() {
        println!("desc:      {}", truncate(&d.description, 400));
    }
    if !d.upc.is_empty() {
        println!("upc:       {}", d.upc);
    }
    println!("url:       {}", d.url());
    for (i, photo) in d.photo_urls().iter().take(3).enumerate() {
        println!("photo {}:   {photo}", i + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_args(argv: &[&str]) -> SearchArgs {
        let mut full = vec!["macbid", "search"];
        full.extend_from_slice(argv);
        match Cli::parse_from(full).command {
            Command::Search { args } => args,
            _ => panic!("expected search subcommand"),
        }
    }

    #[test]
    fn category_only_search_browses_with_a_star() {
        let args = search_args(&["--category", "Televisions"]);
        let filter = build_filter(&args).unwrap();
        assert!(filter.is_browse());
        let pairs = filter.to_query_pairs();
        assert_eq!(
            pairs
                .iter()
                .find(|(k, _)| k == "q")
                .map(|(_, v)| v.as_str()),
            Some("*")
        );
        assert!(filter.filter_by().contains("category:=`Televisions`"));
    }

    #[test]
    fn browse_subcommand_turns_into_a_category_filter() {
        let cli = Cli::parse_from([
            "macbid",
            "browse",
            "--category",
            "Laptops",
            "--price-to",
            "400",
            "--limit",
            "5",
        ]);
        let Command::Browse { category, args } = cli.command else {
            panic!("expected browse");
        };
        let search = args.into_search_args(category);
        let filter = build_filter(&search).unwrap();
        assert!(filter.is_browse());
        assert_eq!(filter.categories, vec!["Laptops"]);
        assert_eq!(filter.price_to, Some(400.0));
        assert_eq!(filter.per_page, Some(5));
    }

    #[test]
    fn multiple_categories_and_locations_are_kept() {
        let args = search_args(&[
            "tv",
            "--category",
            "Televisions,Laptops",
            "--location",
            "Robinson,El Paso",
        ]);
        let filter = build_filter(&args).unwrap();
        assert_eq!(filter.categories, vec!["Televisions", "Laptops"]);
        assert_eq!(filter.locations, vec!["Robinson", "El Paso"]);
        let fb = filter.filter_by();
        assert!(fb.contains("category:=`Laptops`"), "got {fb}");
        assert!(fb.contains("auction_location:=`El Paso`"), "got {fb}");
    }

    #[test]
    fn states_are_uppercased() {
        let args = search_args(&["x", "--state", "pa,oh"]);
        assert_eq!(build_filter(&args).unwrap().states, vec!["PA", "OH"]);
    }

    #[test]
    fn bad_closing_date_is_rejected_before_hitting_the_api() {
        let args = search_args(&["x", "--closing-before", "next tuesday"]);
        assert!(build_filter(&args).unwrap_err().contains("YYYY-MM-DD"));
    }

    #[test]
    fn inverted_price_range_is_rejected() {
        let args = search_args(&["x", "--price-from", "50", "--price-to", "10"]);
        assert!(build_filter(&args).unwrap_err().contains("above"));
    }

    #[test]
    fn pallet_flags_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["macbid", "search", "--pallet", "--include-pallets"]).is_err()
        );
    }

    #[test]
    fn titles_only_keeps_only_title_matches() {
        let lf = local_filter(&search_args(&["lg oled", "--titles-only"]));
        assert_eq!(lf.require_words, vec!["lg", "oled"]);
        assert_eq!(lf.case, CaseMode::IgnoreCase);
    }

    #[test]
    fn default_includes_description_matches() {
        let lf = local_filter(&search_args(&["lg oled"]));
        assert!(lf.require_words.is_empty());
    }

    #[test]
    fn saved_search_roundtrip_keeps_every_filter() {
        let args = search_args(&[
            "--category",
            "Televisions",
            "--titles-only",
            "--include-closed",
            "--discount-min",
            "70",
        ]);
        let back: SearchArgs =
            serde_json::from_str(&serde_json::to_string(&args).unwrap()).unwrap();
        assert!(back.titles_only);
        assert!(back.include_closed);
        assert_eq!(back.discount_min, Some(70.0));
        assert_eq!(back.category, vec!["Televisions"]);
    }

    #[test]
    fn old_saved_searches_without_new_flags_still_load() {
        let legacy = r#"{"query":"tv"}"#;
        let back: SearchArgs = serde_json::from_str(legacy).unwrap();
        assert_eq!(back.query.as_deref(), Some("tv"));
        assert!(!back.titles_only);
    }

    #[test]
    fn close_label_is_mm_dd() {
        let lot: Lot = serde_json::from_value(json!({
            "product_name": "x",
            "expected_close_date": "2026-10-29"
        }))
        .unwrap();
        assert_eq!(closes_label(&lot), "10-29");
    }
}
