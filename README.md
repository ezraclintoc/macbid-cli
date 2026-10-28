# macbid-cli

Search, filter and watch [Mac.Bid](https://www.mac.bid) auctions from your
terminal — or let your AI agent do it via MCP. Built in Rust on top of the same
search index the Mac.Bid web app queries.

**Read-only by design:** this tool only *reads* Mac.Bid. It cannot bid, buy,
list, or touch your account. There is nowhere to log in, because no login is
ever performed.

```console
$ macbid browse --category Televisions --order price-low-to-high --limit 4
$ macbid search "espresso machine" --price-to 200 --condition like-new
$ macbid facets --field category          # what categories exist right now
$ macbid lot 66563445                     # test notes, description, photos
```

## Why it exists

Everything here is about **automating the search**. Mac.Bid's site can filter by
category, location, condition, price and closing date — this exposes those same
facets to a script, a shell pipeline or an agent, adds a keyword blocklist on
top, and remembers what you already saw so a watch only reports *changes*.

The two things the website can't do for you:

* **Search a category with no search text.** `q=*` plus a category facet means
  "show me everything in Televisions", sorted however you like. No made-up
  keyword required.
* **Diff a saved search.** Save it once, run `macbid diff` later, and get only
  new lots and lots that disappeared — instead of re-reading a 40-page grid.

## Quick start

Rust 1.85+:

```console
$ cargo install --path crates/macbid-cli
# or, from a checkout:
$ cargo run -- browse --category Laptops --limit 5
```

Prebuilt binaries aren't published; build from source.

## How to use

Every command takes the global `--json` flag for machine-readable output with
one stable shape per command.

### Search

```console
$ macbid search "lego" --category "Toys & Games" --price-to 60 \
    --order ending-soon --limit 30 --pages 2
```

The `QUERY` is optional: leave it out and you're browsing by filter alone.

| Flag | Meaning |
| --- | --- |
| `QUERY` (positional) | free text, matched against product name, auction title, brand, UPC and keywords |
| `--category NAME` | category facet; repeatable / comma separated |
| `--location NAME` | pickup location (Robinson, El Paso, …); repeatable |
| `--state CODE` | two-letter state; repeatable |
| `--condition open-box\|like-new\|damaged` | lot condition; repeatable |
| `--price-from F`, `--price-to F` | current bid range |
| `--retail-from F`, `--retail-to F` | retail price range |
| `--discount-min PCT` | at least N% off retail (e.g. 80) |
| `--bids-min N`, `--bids-max N` | how contested the lot is (0 = untouched) |
| `--closing-before YYYY-MM-DD`, `--closing-after YYYY-MM-DD` | closing window |
| `--shippable`, `--transferable`, `--turbo`, `--free-transfer`, `--featured` | lot flags |
| `--pallet` | pallets only (pallets are excluded by default, like the site) |
| `--include-pallets` | keep pallets in the mix |
| `--include-closed` | include lots that already closed |
| `--order` | `relevance`, `ending-soon`, `newest`, `price-low-to-high`, `price-high-to-low`, `retail-high-to-low`, `biggest-discount`, `most-bids` |
| `--page N`, `--limit N` (max 250) | pagination |
| `--pages N` | fetch N consecutive pages, merged and de-duplicated |

**Local text filters** (applied after the fetch): `--exclude-text WORDS`
(blocklist), `--titles-only` (the title itself must contain every query word —
the server also matches brand/UPC/keywords), `--case-sensitive` (exact
capitalization).

### Browse a category

The point of the tool: no query text at all.

```console
$ macbid browse --category Televisions --order price-low-to-high --limit 5
$ macbid browse --category "Home & Kitchen" --location Robinson --price-to 40
$ macbid browse --category Laptops --discount-min 85 --order biggest-discount
```

`browse` is sugar for `search --category …` with the query left empty.

### Facets

See the actual values Mac.Bid uses, with live lot counts:

```console
$ macbid facets                          # top values of all four facets
$ macbid facets --field category --limit 40
$ macbid facets --field location --in-category Televisions
```

(Also available as `macbid categories`.)

### Lots

```console
$ macbid lot 66563445
```

Gives title, category, condition, current bid, retail, buy-now price, bids,
closing time, auction/pickup info, the tester's notes ("Powers on. Displays
image."), the seller description, UPC, the public URL and photo links.

### Saved searches & diff

Save a search once, then keep watching it. State lives under your config
directory (`$XDG_CONFIG_HOME/macbid-cli` by default):

```console
$ macbid save tvs --category Televisions --limit 40 --order newest
saved search `tvs` — run it and diff with: macbid diff tvs
$ macbid diff tvs            # first run records a baseline
$ macbid diff tvs            # after a while: what changed
+ added (2):
  [66940920] Samsung 65" OLED — $412.00 — Robinson — 10-28
- gone (1):
  [66783845] Onn FHD Monitor — $31.00 — Rock Hill — 10-27
37 lots unchanged
$ macbid searches            # list saved searches
$ macbid forget tvs          # remove it + its state
```

“Gone” means the lot stopped appearing in the results — closed, sold to someone
else, or pushed out of the result window between runs. Use `diff --pages N` for
deeper coverage and `--export FILE.csv` to get rows out.

## AI agent (MCP)

`macbid` is also a **[Model Context Protocol](https://modelcontextprotocol.io)
server**. Install it into your harness once and the agent gets seven tools:

| Tool | What it does |
| --- | --- |
| `search_lots` | full search: text (optional), every facet, ordering, pagination |
| `browse_category` | a whole category with no query text |
| `list_facets` | category / location / condition / state values with counts |
| `lot_details` | description, test notes, photos, bids for one lot |
| `list_watches` | saved searches on this machine |
| `watch_diff` | new / gone lots since the last run |
| `watch_save` | create or overwrite a watch the agent can poll |

They share the CLI's search pipeline and the same saved-search files: save a
watch in the shell, poll it from the agent (or vice versa).

Register it by adding one entry — command `macbid`, args `["mcp", "serve"]`, no
host or port (stdio is local):

```jsonc
// Claude Code (~/.claude.json), Cursor, Windsurf, Freebuff (.agents/mcp.json)
{
  "mcpServers": {
    "macbid": { "command": "macbid", "args": ["mcp", "serve"] }
  }
}
```

```toml
# Codex (~/.codex/config.toml)
[mcp_servers.macbid]
command = "macbid"
args = ["mcp", "serve"]
```

## Where the data comes from

Mac.Bid runs an Auction Mobility storefront. Its search box queries a Typesense
index (`search-1..5.mac.bid`, collection `prod_macdiscount_alias`), and the
read-only search key is the one the public web bundle ships. This tool queries
that index directly, which is why the facets, ordering and result counts match
what the site shows — the site's own filter UI is just a client of the same
index. Lot details come from the storefront's public `/lot/<id>` endpoint.

Overrides, if anything upstream ever rotates:

| Variable | Default |
| --- | --- |
| `MACBID_TYPESENSE_KEY` | the public read key from the mac.bid bundle |
| `MACBID_TYPESENSE_NODES` | `https://search-1..5.mac.bid` (comma separated) |
| `MACBID_TYPESENSE_COLLECTION` | `prod_macdiscount_alias` |
| `MACBID_API_BASE` | `https://api.macdiscount.com` |

## Caveats

**Be polite.** Mac.Bid has no documented public API. This tool makes one request
per search, one per `--pages` page, and one per `lot` lookup (two: the detail
page plus one index query for the current bid). It rotates across their five
search nodes on failure and does not hammer. Heavy automated use may get your IP
throttled — and none of this is sanctioned by Mac.Bid.

**Search results only carry what the index holds.** Descriptions, test notes and
full photo lists require `macbid lot <id>`, which is a second request. A search
result's `image_url` is the product image, not the actual unit.

**Pallets are excluded by default.** That matches the site's search box; use
`--pallet` or `--include-pallets` to change that.

## Legal notice

This is an **open-source community project, not affiliated with, endorsed by, or
sponsored by Mac.Bid / Mac Discount** in any way. Source is provided as-is,
without warranty of any kind; use of it is subject to Mac.Bid's Terms of Service.
If Mac.Bid asks this project to stop, it will comply.

## License

MIT — see [LICENSE](LICENSE).
