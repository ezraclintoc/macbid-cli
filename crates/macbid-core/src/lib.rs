//! `macbid-core` — async Rust client for Mac.Bid's search API.
//!
//! Mac.Bid runs an Auction Mobility storefront whose search box is backed by a
//! Typesense index (`search-1..5.mac.bid`, collection `prod_macdiscount_alias`).
//! The read-only search key is the one the public web app ships, and the index
//! exposes Mac.Bid's own facets — category, auction location, state, condition,
//! price, closing date and the pallet/turbo/shipping flags — so filtering here
//! is the same filtering the site itself does, just scriptable.
//!
//! This crate is intentionally independent of any CLI/UI layer so it can be
//! reused by an MCP server or a web frontend later. Every request is a read.

pub mod client;
pub mod error;
pub mod local_filters;
pub mod models;

pub use client::{Facets, MacBidClient, MergedSearch};
pub use error::{Error, Result};
pub use local_filters::{CaseMode, LocalFilter};
pub use models::{Condition, FacetCount, Lot, LotDetail, SearchFilter, SearchResponse, SortOrder};
