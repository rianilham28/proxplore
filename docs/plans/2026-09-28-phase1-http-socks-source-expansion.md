# Phase 1: HTTP/SOCKS Source Expansion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) with dispatching-parallel-agents for independent tasks to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the remaining keyless plain-GET HTTP/SOCKS lanes (sub-lane deepening + new provider + catalog sweep) into proxplore without touching the proxalyze consumer contract.

**Architecture:** Two parallel waves. Wave 1 adds `Request` rows to existing providers and one new single-module provider, each gated by a live probe and a drop-on-failure rule. Wave 2 re-verifies every catalog row and discovers new sources, then wires live plain-GET findings by the same rules. All lanes ride the existing `Provider`/`ParseKind`/`fetch.rs` stack unchanged.

**Tech Stack:** Rust 2024 (MSRV 1.98), tokio, wreq, clap, regex/serde_json (all in-tree, no new deps), curl for read-only probes.

**Spec:** `docs/superpowers/specs/2026-09-28-phase1-http-socks-source-expansion-design.md`

## Global Constraints

- Plain-GET only: no changes to `src/model.rs`, `src/normalize.rs`, `src/fetch.rs`, `src/runner.rs`, or shared `src/parse.rs` beyond what a task names explicitly (none do).
- Keyless-only: no tokens, no signup endpoints (Webshare excluded), no gated feeds.
- `.jsonl` / `.summary.json` schemas and exit codes 0/1/2/130 unchanged.
- No new Cargo dependencies; parsers use in-tree `regex`/string handling.
- CI gates each task must pass: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked`.
- Commit identity: `rianilham <rianilham28@users.noreply.github.com>` (repo-local config, already pinned).
- Drop-on-failure: a lane failing its probe or smoke is removed from the registry, never shipped silent.
- Never force-push; conventional commit subjects (lowercase, imperative, no trailing period).

**Evidence-artifact conventions (verified against `src/runner.rs`, use in every smoke/acceptance step):**
- Provenance file: `<stem>.provenance.jsonl` (e.g. `--output /tmp/fpl.txt` → `/tmp/fpl.provenance.jsonl`), never `<stem>.jsonl`.
- Provenance line schema: `{"proxy": "<url>", "source": "<provider-id>", "run_started_at": "..."}` — field is **`source`**, and per-line contribution is proven by `source == "<id>"` counts; `Request.label` is never persisted (it exists only in logs), so label-grep is forbidden.
- Summary file: `<stem>.summary.json`; outcome lives under **`outcome`** (`full`/`partial`/...), plus `exit_code`, `records_total`, `records_unique`, and `providers[]` entries of `{id, ok_requests, total_requests, records, duration_secs, truncated, errors}`.
- Per-lane record counts come from `providers[].records` in a `--providers <id>` run, or from `source`-grouped provenance counts in a full run.

## Review Focus

1. **proxyscrape `ssl` filter** — catalog documents `ssl` as a separate param from `protocol` (which only takes http/socks4/socks5); an https pool is by construction a *subset* of http. Expect: probe `protocol=http&ssl=true` vs `protocol=http`; pass condition = non-empty strict subset (a legitimately distinct slice), NOT distinct counts. Wrong param spelling → empty result → lane dropped.
2. **us-proxy textarea absence** — if `us-proxy.html` lacks the raw `<textarea>` block, `parse_textarea` returns 0 records on a 200 response (silent empty lane). Expect: probe greps for `<textarea` before wiring; smoke asserts >0 records.
3. **spys mirror coverage** — `spys.me` may expose only `proxy.txt`; guessing sibling URLs (e.g. `socks.txt`) risks 404 lanes. Expect: probe each candidate URL for HTTP 200 + parseable rows before adding `Request` rows; only surviving URLs wired.
4. **Scrappey extraction shape** — verified 2026-09-28: rows are `<tr data-protocol="http|socks4|socks5" data-search="ip:port">` cells (table) plus a `window.__SSR_DATA__` JSON block; all 180 rows are stale `fallback`/`likelyDead:true` (2026-09-23) — quality is proxalyze's call, not a drop condition. Expect: table-cell parser reading `data-protocol` per row (NOT a default-http assumption — pool is 60/60/60 http/socks4/socks5); fixture built from the observed row shape; parser returning 0 rows on live smoke → drop.
5. **roosterkid decay** — 6 rows, possibly stale/empty. Expect: probe default branch via `git ls-remote --symref` first (a wrong branch is indistinguishable from decay in a 404 body), then count: <10 non-comment lines → skip wiring (documented in catalog update instead).

---

### Task 0: Record pre-change baselines

**Files:** none (read-only; evidence recorded in execution notes for later tasks).

**Interfaces:**
- Consumes: existing registry.
- Produces: two numbers every later task's evidence compares against — registry count and per-provider record counts.

- [ ] **Step 1: Capture registry and per-provider baselines**

Run:
```bash
cargo run --release -- --list-providers | wc -l | tee /tmp/baseline-registry-count.txt
cargo run --release --output /tmp/baseline.txt >/dev/null 2>&1; echo "exit=$?"
python3 -c "import json,collections;c=collections.Counter(json.loads(l)['source'] for l in open('/tmp/baseline.provenance.jsonl'));[print(k,v) for k,v in sorted(c.items())]" | tee /tmp/baseline-source-counts.txt
```
Expected: registry count written (this is the measured baseline — README's "30" is a claim, not evidence); full harvest exit 0 or 2; per-`source` counts saved for `free-proxy-list-net`, `proxyscrape`, `spys` (they anchor Task 1/2/3 lane-delta proofs).

- [ ] **Step 2: No commit (read-only evidence).**

---

### Task 1: free-proxy-list `us-proxy.html` lane

**Files:**
- Modify: `src/providers/free_proxy_list.rs` (request plan ~line 103-110, tests module appended)

**Interfaces:**
- Consumes: `parse_textarea(body, default) -> Vec<ProxyRecord>` (existing, same file), `Request::new(url, label).with(scheme, parser)` from `src/model.rs`.
- Produces: registry id unchanged (`free-proxy-list-net`); one new `Request` labeled `"us"` with `Some(Scheme::Http)` and `ParseKind::Custom(parse_textarea)`. No other task depends on this lane.

- [ ] **Step 1: Probe the page for the textarea block (read-only)**

Run:
```bash
curl -sS -o /tmp/us-proxy.html -w '%{http_code}\n' https://free-proxy-list.net/us-proxy.html && grep -c '<textarea' /tmp/us-proxy.html
```
Expected: `200` then a count ≥ `1`. **If this fails: SKIP Steps 3-8 entirely (no test, no code, no commit) — record "us-proxy lane dropped: no textarea block" and proceed to Task 2.** A dropped lane must leave no failing test behind.

- [ ] **Step 2: Verify probe output contains parseable rows**

Run:
```bash
grep -oE '([0-9]{1,3}\.){3}[0-9]{1,3}:[0-9]{2,5}' /tmp/us-proxy.html | head -5
```
Expected: ≥5 `ip:port` lines. If this fails → same drop rule as Step 1: skip Steps 3-8, record the drop, proceed to Task 2 (the drop rule: no lane without live rows).

- [ ] **Step 3: Write the failing test**

Append to the existing `#[cfg(test)] mod tests` in `src/providers/free_proxy_list.rs` (file currently has **no** tests module — create it at EOF following `e89ip.rs:90-114` structure):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_cover_main_ssl_socks_and_us_pages() {
        let urls: Vec<String> = FreeProxyListNet.requests().iter().map(|r| r.url.clone()).collect();
        assert_eq!(urls, [
            "https://free-proxy-list.net/",
            "https://free-proxy-list.net/ssl-proxy.html",
            "https://free-proxy-list.net/socks-proxy.html",
            "https://free-proxy-list.net/us-proxy.html",
        ]);
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --locked free_proxy_list`
Expected: FAIL — `assertion failed` on URL list length (3 vs 4) or compile error on missing tests module is acceptable only as the red state.

- [ ] **Step 5: Add the request row**

In `fn requests()` in `src/providers/free_proxy_list.rs`, append after the `"socks"` row:

```rust
Request::new("https://free-proxy-list.net/us-proxy.html", "us")
    .with(Some(Scheme::Http), ParseKind::Custom(parse_textarea)),
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --locked free_proxy_list`
Expected: PASS.

- [ ] **Step 7: Live smoke the lane**

Run: `cargo run --release -- --providers free-proxy-list-net --output /tmp/fpl.txt; echo exit=$?; wc -l < /tmp/fpl.txt; python3 -c "import json,collections;print(collections.Counter(json.loads(l)['source'] for l in open('/tmp/fpl.provenance.jsonl')))"`
Expected: exit 0 or 2; output non-empty; provenance `source` counts show `free-proxy-list-net` records — and to isolate the *new lane's* contribution, additionally run `--output /tmp/us.txt` and confirm `/tmp/us.provenance.jsonl` line count > the pre-change baseline recorded in Task 0 (all four lanes share one source id, so lane-level proof is baseline-delta, not source grouping).

- [ ] **Step 8: Commit**

Run: `git add src/providers/free_proxy_list.rs && git commit -m "feat: add us-proxy subpage lane to free-proxy-list provider"`

---

### Task 2: proxyscrape `https` lane

**Files:**
- Modify: `src/providers/proxyscrape.rs` (const `PROTOCOLS` line 14-18, `protocols()` line 33-35, tests module at EOF)

**Interfaces:**
- Consumes: existing `PROTOCOLS` table drives `requests()` — adding a row automatically emits one more `Request`; no other code change needed.
- Produces: `protocols()` string becomes `"http,https,socks4,socks5"` (registry display contract for `--list-providers`).

- [ ] **Step 1: Probe the `ssl` filter for the https slice (read-only)**

Run:
```bash
curl -sS 'https://api.proxyscrape.com/v4/free-proxy-list/get?request=display_proxies&protocol=http&proxy_format=ipport&format=text' -o /tmp/ps-http.txt
curl -sS 'https://api.proxyscrape.com/v4/free-proxy-list/get?request=display_proxies&protocol=http&ssl=true&proxy_format=ipport&format=text' -o /tmp/ps-ssl.txt
wc -l /tmp/ps-http.txt /tmp/ps-ssl.txt
```
Expected: `ssl=true` result **non-empty** (if empty → param spelling wrong or filter yields nothing → drop lane, skip to Task 3). Note: `protocol` deliberately stays `http` — catalog §1 documents `ssl` as a separate filter and `protocol` only takes http/socks4/socks5; do NOT try `protocol=https`.

- [ ] **Step 2: Verify the ssl slice is a subset of http (correctness, not distinctness)**

Run: `comm -23 <(sort -u /tmp/ps-ssl.txt) <(sort -u /tmp/ps-http.txt) | wc -l`
Expected: `0` — every ssl-filtered row must also appear in the unfiltered http pool (the ssl slice is a subset by construction; rows *outside* the http pool would mean the filter semantics differ from documented and the labeled `https` scheme claim is unsupported → drop lane, skip to Task 3). Then record `sort -u /tmp/ps-ssl.txt | wc -l` — if it equals the http unique count (filter ignored, identical pools) → also drop: no distinct slice, zero new volume.

- [ ] **Step 3: Write the failing test**

Append at EOF of `src/providers/proxyscrape.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_cover_http_https_slice_and_socks_lanes() {
        let provider = ProxyScrape;
        let labels: Vec<&str> = provider.requests().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["http", "https", "socks4", "socks5"]);
        assert_eq!(provider.protocols(), "http,https,socks4,socks5");
        let https_req = &provider.requests()[1];
        assert_eq!(
            https_req.url,
            format!("{API}?request=display_proxies&protocol=http&ssl=true&proxy_format=ipport&format=text")
        );
        assert_eq!(https_req.scheme, Some(Scheme::Https));
        assert_eq!(https_req.label, "https");
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --locked proxyscrape`
Expected: FAIL — labels are 3 entries, `https` missing.

- [ ] **Step 5: Restructure the table to query fragments and add the ssl lane**

The https lane differs only by an extra query param, so the table's second field becomes the full protocol query fragment:

```rust
const PROTOCOLS: [(Scheme, &str, &str); 4] = [
    (Scheme::Http, "http", "protocol=http"),
    (Scheme::Https, "https", "protocol=http&ssl=true"),
    (Scheme::Socks4, "socks4", "protocol=socks4"),
    (Scheme::Socks5, "socks5", "protocol=socks5"),
];
```

Update `requests()` to destructure `(scheme, label, query)` and build `format!("{API}?request=display_proxies&{query}&proxy_format=ipport&format=text")` with label `*label` (keeps the existing URL shape byte-identical for the three current lanes — covered by the test above asserting the exact https URL; add no other URL assertions). Change `fn protocols()` to `"http,https,socks4,socks5"`.

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --locked proxyscrape`
Expected: PASS.

- [ ] **Step 7: Live smoke the lane**

Run: `cargo run --release -- --providers proxyscrape --output /tmp/ps.txt; echo exit=$?; grep -c '^https://' /tmp/ps.txt`
Expected: exit 0; https-prefixed records > 0.

- [ ] **Step 8: Commit**

Run: `git add src/providers/proxyscrape.rs && git commit -m "feat: add https lane to proxyscrape provider"`

---

### Task 3: spys sub-list mirrors (probe-gated, likely conditional)

**Files:**
- Modify: `src/providers/spys.rs` (request plan line 29-33, tests module at EOF)

**Interfaces:**
- Consumes: `ParseKind::Entries` shared component; `Request::new(...).with(Some(Scheme::Http), ParseKind::Entries)` pattern already in the file.
- Produces: possibly zero change if no mirrors survive the probe — that outcome is recorded, not a failure.

- [ ] **Step 1: Probe candidate mirror URLs (read-only)**

Run:
```bash
for u in socks.txt https.txt socks5.txt proxy-socks.txt; do
  printf '%s ' "$u"
  curl -sS -o "/tmp/spys-$u" -w '%{http_code}\n' "http://spys.me/$u" || echo "err"
done
```
Expected per URL: HTTP `200` AND body containing ≥10 `ip:port`-shaped tokens:
```bash
grep -oE '([0-9]{1,3}\.){3}[0-9]{1,3}:[0-9]{2,5}' "/tmp/spys-$u" | wc -l
```
404/403/empty → that candidate is dead; do not wire it.

- [ ] **Step 2: Decide the lane set**

If **zero** candidates survive: record "no spys mirrors beyond proxy.txt" in execution notes, **SKIP Steps 3-8 entirely (no test, no code, no commit)** and proceed to Task 4. If ≥1 survives: continue, wiring exactly the surviving URLs.

- [ ] **Step 3: Write the failing test**

Append at EOF of `src/providers/spys.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_txt_mirror_with_http_default() {
        let requests = Spys.requests();
        assert_eq!(requests.len(), 1); // grows only when a probe-verified mirror survives
        assert_eq!(requests[0].url, "http://spys.me/proxy.txt");
        assert_eq!(requests[0].label, "txt");
        assert!(matches!(requests[0].parser, ParseKind::Entries));
    }
}
```

If Step 2 wired N mirrors, the assertion becomes `len() == 1 + N` with each new URL/label listed — the implementer updates the expected list to match exactly what was probed, never a guessed URL.

- [ ] **Step 4: Verify the test is red before implementing**

If mirrors survived Step 2, the Step 3 test (asserting `len() == 1 + N` with the surviving URLs) must FAIL against the current single-row `requests()` — run `cargo test --locked spys` and confirm FAIL. If no mirrors survived, this task ended at Step 2.

- [ ] **Step 5: Add surviving mirror requests**

In `fn requests()` append rows for each surviving URL, e.g.:

```rust
Request::new("http://spys.me/socks.txt", "socks")
    .with(Some(Scheme::Http), ParseKind::Entries),
```
(label = mirror stem; scheme http unless the probe body shows scheme-prefixed lines, in which case `None`.)

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --locked spys`
Expected: PASS.

- [ ] **Step 7: Live smoke the lane**

Run: `cargo run --release -- --providers spys --output /tmp/spys.txt; echo exit=$?; wc -l < /tmp/spys.txt; python3 -c "import json;print(sum(1 for l in open('/tmp/spys.provenance.jsonl')))"`
Expected: exit 0 or 2; provenance line count > the `/tmp/baseline-source-counts.txt` spys entry from Task 0 (lane-level proof is baseline delta — `Request.label` is never persisted, so there is nothing label-specific to grep).

- [ ] **Step 8: Commit**

Run: `git add src/providers/spys.rs && git commit -m "feat: add probe-verified spys mirror sublists"`

---

### Task 4: Scrappey provider (new single module)

**Files:**
- Create: `src/providers/scrappey.rs`
- Modify: `src/providers/mod.rs` (add `mod scrappey;` under the HTML section and `scrappey::new(),` in `all()`)

**Interfaces:**
- Consumes: `Provider` trait (`src/model.rs:127-183`), `make_proxy` + `Scheme::from_label` (imports per `free_proxy_list.rs`), `ParseKind::Custom`.
- Produces: registry id `"scrappey"`; `Custom` parser `fn parse_rows(body: &str, default: Option<Scheme>) -> Vec<ProxyRecord>` (provider-local, same shape as `ParseFn`).

**Structure note (verified live 2026-09-28):** the page carries the rows in TWO forms: (a) a server-rendered `<tbody>` of `<tr data-protocol="http|socks4|socks5" data-search="ip:port">` rows whose first two `<td>` cells are ip and port (180 data rows, 60/60/60 across protocols, plus one `<th>` header row), and (b) a `window.__SSR_DATA__ = {"proxyData":{"proxies":[...]}}` JSON block with the same data. Plan uses (a) — it matches the in-repo table-parser pattern — with (b) as the probe-verified fallback. All 180 rows are `likelyDead:true`/`source:"fallback"` (lastChecked 2026-09-23): stale-flagged, but proxplore's charter is to present what sources publish; judging is proxalyze's job — NOT a drop condition.

- [ ] **Step 1: Probe the page for table rows (read-only)**

Run:
```bash
curl -sS -o /tmp/scrappey.html -w '%{http_code}\n' --max-time 20 https://scrappey.com/tools/free-proxy-lists/socks5
grep -c 'data-protocol=' /tmp/scrappey.html
```
Expected: `200` and `data-protocol` count ≥ 10. **If 0: check the fallback** — `grep -c '__SSR_DATA__' /tmp/scrappey.html` ≥ 1 → switch to Step 5b below (serde extraction) and skip the table branch; both 0 → **drop the provider: SKIP Steps 3-10, record the drop, proceed to Task 5.**

- [ ] **Step 2: Confirm per-row scheme attributes and cell shape (read-only)**

Run: `grep -oE '<tr data-protocol="[a-z0-9]+"[^>]*data-search="[0-9.]+' /tmp/scrappey.html | head -3`
Expected: rows where `data-protocol` ∈ {http, socks4, socks5} and `data-search` starts with an ip — confirms scheme is per-row in the attribute (a flat `Some(Scheme::Http)` default would mislabel 120 of 180 rows; the parser must read the attribute).

- [ ] **Step 3: Write the failing test**

Create `src/providers/scrappey.rs` (structure per `e89ip.rs:90-114`), fixture copied from the observed row shape (classes can be shortened to the essentials; tag structure must match Step 1-2 output):

```rust
//! Scrappey — single-page free proxy list tool. Rows are server-rendered
//! <tr data-protocol=...> cells; protocol is a per-row attribute (60/60/60
//! http/socks4/socks5), not a page-level default. Rows carry a stale
//! likelyDead flag upstream; we publish them as-is (proxalyze judges).

use std::sync::Arc;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;

const PAGE: &str = "https://scrappey.com/tools/free-proxy-lists/socks5";

fn parse_rows(body: &str, _default: Option<Scheme>) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for row in body.split("<tr ").skip(1) {
        // rows without data-protocol (the <th> header row) fall through
        /* extract data-protocol value, first two <td> contents = ip, port;
           Scheme::from_label(protocol)?, make_proxy(scheme, ip, port,
           None, None, "scrappey"), push on success */
    }
    out
}

pub struct Scrappey;

impl Provider for Scrappey {
    fn id(&self) -> &'static str { "scrappey" }
    fn site(&self) -> String { PAGE.into() }
    fn protocols(&self) -> String { "http,socks4,socks5".into() }
    fn refresh(&self) -> &'static str { "~15 min" }
    fn requests(&self) -> Vec<Request> {
        // None: every row self-describes its protocol via data-protocol
        vec![Request::new(PAGE, "list").with(None, ParseKind::Custom(parse_rows))]
    }
}

pub fn new() -> Arc<dyn Provider> { Arc::new(Scrappey) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rows_reads_per_row_protocol_and_skips_header() {
        let body = concat!(
            "<table><thead><tr><th>IP</th><th>Port</th></tr></thead><tbody>",
            "<tr data-protocol=\"http\" data-search=\"8.8.8.8:8080\" class=\"x\">",
            "<td>8.8.8.8</td><td>8080</td><td></td></tr>",
            "<tr data-protocol=\"socks5\" data-search=\"1.1.1.1:3128\">",
            "<td>1.1.1.1</td><td>3128</td></tr>",
            "<tr data-protocol=\"socks4\" data-search=\"9.9.9.9:1080\">",
            "<td>9.9.9.9</td><td>1080</td></tr>",
            "</tbody></table>",
        );
        assert_eq!(
            parse_rows(body, None).into_iter().map(|r| r.url()).collect::<Vec<_>>(),
            ["http://8.8.8.8:8080", "socks5://1.1.1.1:3128", "socks4://9.9.9.9:1080"]
        );
    }
}
```

(The `parse_rows` body is the one source-specific piece — filled in from Step 1-2 output; the test pins the contract: per-row scheme from the attribute, header row skipped.)

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --locked scrappey`
Expected: FAIL (red) — `parse_rows` returns empty vec vs expected URLs.

- [ ] **Step 5: Implement `parse_rows` (table branch)**

Complete the body: for each `<tr ` segment after the first, read the `data-protocol="…"` attribute (skip the row if absent), `Scheme::from_label(protocol)`, extract the first two `<td>` contents as ip/port, `make_proxy(scheme, ip, port, None, None, "scrappey")`, push on `Some`. Cell extraction follows `free_proxy_list.rs::parse_socks_columns`' `<td`-split idiom.

- [ ] **Step 5b: Fallback — SSR JSON extraction (only if Step 1 chose it)**

Extract the `window.__SSR_DATA__ = {…}` object (up to `</script>`), `serde_json::from_str`, walk `["proxyData"]["proxies"]`, build records from each element's `ip`/`port`/`protocol` fields with `serde_json::Value::as_str`/`as_u64` guards; on missing marker or parse error return an empty vec (graceful, never panic — the drain layer reports 0 records). Same test contract; fixture becomes a minimal JSON string instead of HTML. No new dependency: `serde_json` is already in `Cargo.toml`.

- [ ] **Step 6: Register the provider and refresh the module doc counts**

In `src/providers/mod.rs`:
1. Add `mod scrappey;` in the HTML list-page section and `scrappey::new(),` to `all()` (alphabetical position among the bespoke modules).
2. Update the file's header doc-comment counts — "The 16 cron-refreshed GitHub raw repos" and "the 14 sources with real logic" become stale once scrappey registers (15 if no other bespoke provider was added; recount from the actual module lists in the file, don't trust these numbers).

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo test --locked scrappey && cargo test --locked`
Expected: PASS (both the module test and the full suite — registration guard `guard_registry_integrity` tests in main.rs run against `all()`).

- [ ] **Step 8: Verify registration is visible**

Run: `cargo run --release -- --list-providers | grep -i scrappey`
Expected: one line with id, site URL, protocols.

- [ ] **Step 9: Live smoke the provider**

Run: `cargo run --release -- --providers scrappey --output /tmp/scrappey.txt; echo exit=$?; wc -l < /tmp/scrappey.txt; python3 -c "import json;print(sum(1 for l in open('/tmp/scrappey.provenance.jsonl')))"`
Expected: exit 0; provenance line count > 0 (drop the provider — remove `mod scrappey;` and `scrappey::new(),` from `mod.rs`, delete the file, no commit — if the live parse yields 0: parser/structure mismatch means Steps 1-2 observed something different from the fixture).

- [ ] **Step 10: Commit**

Run: `git add src/providers/scrappey.rs src/providers/mod.rs && git commit -m "feat: add scrappey tool page provider"`

---

### Task 5: roosterkid feed re-verification (decision task)

**Files:**
- Modify (conditional): `src/providers/github_feeds.rs` — only if the probe passes
- No file changes if skipped

**Interfaces:**
- Consumes: `gh!` macro (`github_feeds.rs:9-26`), `FeedFile::scheme_and(Scheme::Socks5, "SOCKS5_RAW.txt")`.
- Produces: registry id `"roosterkid"` only if wired.

- [ ] **Step 1: Probe default branch and row count (read-only)**

Run:
```bash
git ls-remote --symref https://github.com/roosterkid/openproxylist HEAD | head -1
```
Expected: `ref: refs/heads/<branch> HEAD` — record the branch (commonly `main`, but DO NOT assume; a wrong branch in the `gh!` row silently 404s the lane while the repo is alive). Then:
```bash
curl -sS -o /tmp/roosterkid.txt -w '%{http_code}\n' "https://raw.githubusercontent.com/roosterkid/openproxylist/<branch>/SOCKS5_RAW.txt"
grep -cvE '^\s*(#|$)' /tmp/roosterkid.txt
```
Expected verdicts: HTTP `200` AND ≥10 data lines → continue to Step 2 with the probed branch in the `gh!` snippet; HTTP ≠ 200 → **source gone** (or file renamed — check the repo tree once via `https://github.com/roosterkid/openproxylist` before concluding), skip to Task 6; 200 but <10 lines → **decay confirmed**, skip to Task 6 (per spec: decayed rows documented in catalog instead).

- [ ] **Step 2: Write the failing test (only if wired)**

Append to `github_feeds.rs` tests (create `#[cfg(test)] mod tests` at EOF if absent — grep shows no test module exists):

```rust
#[test]
fn roosterkid_table_row_declares_socks5_raw_file() {
    let ids: Vec<&str> = crate::providers::all().iter().map(|p| p.id()).collect();
    assert!(ids.contains(&"roosterkid"));
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --locked roosterkid`
Expected: FAIL — id absent from registry.

- [ ] **Step 4: Add the gh! row**

In `github_feeds.rs` after the `fate0` row, with the branch **from Step 1's `ls-remote` probe** (the `"main"` below is a placeholder — substitute the probed value):

```rust
gh!(
    roosterkid,
    "roosterkid",
    "roosterkid/openproxylist",
    "<probed-branch>",
    "occasional",
    E,
    false,
    [FeedFile::scheme_and(Scheme::Socks5, "SOCKS5_RAW.txt")]
);
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --locked`
Expected: PASS.

- [ ] **Step 6: Commit (only if wired)**

Run: `git add src/providers/github_feeds.rs && git commit -m "feat: add roosterkid socks5 feed"`

---

### Task 6: Wave 2 — catalog re-verification sweep

**Files:**
- Modify: `free-proxy-sources.md` (Status column of every row in §1, §2, §3, §10 log refresh)

**Interfaces:**
- Consumes: every `Entry point` URL in the catalog.
- Produces: a verified-as-of-2026-09-28 status per row; findings list for Task 7. No code.

- [ ] **Step 1: Batch re-fetch every catalog entry point (read-only)**

Extract entry-point URLs from the catalog tables (§1 five APIs, §2 HTML pages, §3 GitHub raw files — skip §8 dead registry, it stays dead) and probe each with status + first-byte check:

```bash
curl -sS -o /dev/null -w '%{http_code} %{url_effective}\n' --max-time 15 <url>
```
Run as a loop over the extracted list; save output to `/tmp/sweep-status.txt`.

- [ ] **Step 2: Classify and update Status columns**

For each row: `2xx` with body containing data → **live** (date it 2026-09-28); `403/429/5xx/challenge` → **blocked** (recheck §8's browser-required list membership); `DNS fail/404/NXDOMAIN` → **dead** (move note to §8 registry per existing convention). Update the §10 verification log header date and any row whose status changed.

- [ ] **Step 3: Verify the catalog's own consistency**

Run: `grep -c '\*\*live' free-proxy-sources.md && grep -c '\*\*dead' free-proxy-sources.md`
Expected: counts reported in execution notes; every §1-§3 row has exactly one status marker (manual spot-check of the three tables).

- [ ] **Step 4: Commit**

Run: `git add free-proxy-sources.md && git commit -m "docs: re-verify catalog statuses for 2026-09-28"`

---

### Task 7: Wave 2 — new-source discovery and conditional wiring

**Files:**
- Modify: `free-proxy-sources.md` (new rows in §1/§2/§3 as found)
- Modify: whichever provider file the finding belongs to (request row) or new module + `mod.rs` (full provider), following Task 1/Task 4 patterns exactly

**Interfaces:**
- Consumes: Task 6's verified status set; `Request`/`ParseKind`/`gh!` patterns from Tasks 1-5.
- Produces: 0..N new lanes, each probe-gated; zero findings is a valid, documented outcome.

- [ ] **Step 1: Research for keyless plain-GET sources not in the catalog**

Search for TXT/JSON/CSV proxy APIs and GitHub raw proxy feeds published after 2026-09-12 (the catalog's sweep date) plus any lanes missed then. Hard filters: keyless, plain GET, HTTP/HTTPS/SOCKS4/SOCKS5 (no node formats — spec boundary), not in §8 dead registry, not in §4/§5/§6.

- [ ] **Step 2: Probe each candidate (read-only)**

Same two-command pattern as Task 1 Steps 1-2 (HTTP status + ≥10 parseable ip:port tokens). Record survivors with their exact entry URL, format, protocols, cadence.

- [ ] **Step 3: Catalog the findings**

Add surviving candidates as rows to the appropriate §1/§2/§3 table with status **live** and today's date; rejected candidates get a one-line reason (blocked/dead/not-keyless/wrong-format). Commit: `git add free-proxy-sources.md && git commit -m "docs: add phase-1 discovery sweep findings"` (skip if zero new rows — note "no new sources found" instead).

- [ ] **Step 4: Wire each survivor (0..N lanes)**

Per survivor, follow the matching existing pattern — GitHub raw feed → `gh!` row (Task 5 Step 4 shape, and Task 5 Step 1's branch-first probe rule applies: `ls-remote` the default branch, never hardcode); JSON/TXT API → module per `proxyscrape.rs`/`geonode.rs`; HTML page → module per Task 4. Include the pattern-appropriate unit test (URL-construction or fixture parser test), registration in `mod.rs` if new, and `cargo test --locked` after each lane. **One commit per lane**, subject `feat: add <id> provider` or `feat: add <id> lane`.

- [ ] **Step 5: Live smoke each new lane**

Run per lane: `cargo run --release -- --providers <id> --output /tmp/<id>.txt; echo exit=$?; wc -l < /tmp/<id>.txt`
Expected: exit 0, >0 lines; any 0-line lane → remove it (its Task 4-style drop: revert the lane's commit) and record the drop in the catalog row (status **blocked** or **dead** per the observed failure).

---

### Task 8: Full-harvest acceptance and docs

**Files:**
- Modify: `README.md` (provider count line 11-24, lane mentions)

**Interfaces:**
- Consumes: registry as of all prior tasks; `.jsonl`/`.summary.json` contract from README "CI / consumer contract" section.
- Produces: phase-1 acceptance evidence.

- [ ] **Step 1: Full default harvest, contract check**

Run:
```bash
cargo run --release --output /tmp/proxies.txt; echo exit=$?
python3 -c "
import json
lines = [json.loads(l) for l in open('/tmp/proxies.provenance.jsonl')]
s = json.load(open('/tmp/proxies.summary.json'))
print('provenance lines:', len(lines), 'fields:', sorted(lines[0].keys()))
print('outcome:', s['outcome'], 'exit_code:', s['exit_code'], 'unique:', s['records_unique'])"
```
Expected: shell exit == `s['exit_code']`, both 0 or 2 (0 if no lane truncated); every provenance line has exactly `proxy`/`source`/`run_started_at` keys; summary has `outcome` (not `status`) with value `full` when exit 0. Any exit 1 with a data-write failure, or schema drift in those keys → investigate before proceeding (contract is frozen by spec).

- [ ] **Step 2: Registry sanity**

Run: `cargo run --release -- --list-providers | wc -l`
Expected: count equals the Task 0 baseline + (newly wired providers) − (providers dropped during this plan); every lane-level addition inside an existing provider (free-proxy-list, proxyscrape, spys) does NOT change this count — the delta in execution notes names each added/removed id explicitly, compared against `/tmp/baseline-registry-count.txt`, never against README's "30".

- [ ] **Step 3: Per-provider contribution check (acceptance criterion 1)**

Run:
```bash
python3 -c "
import json,collections
c=collections.Counter(json.loads(l)['source'] for l in open('/tmp/proxies.provenance.jsonl'))
[print(k,v) for k,v in c.most_common()]"
```
Expected: every shipped **provider id** shows >0 records; new/changed providers (`scrappey`, `roosterkid`, `free-proxy-list-net`, `proxyscrape`, `spys`) show ≥ their Task 0 baseline counts from `/tmp/baseline-source-counts.txt` (lane-level deltas for providers sharing an id). Any newly added provider at 0 → remove it from the registry and re-run (drop rule).

- [ ] **Step 4: Update README**

Update `README.md` §Scope: provider count ("30 identified providers" → actual), add newly wired sources to the lane bullets (API lane list, HTML lane list, GitHub feed list as applicable), and refresh the catalog date reference if the §10 log date changed.

- [ ] **Step 5: Full CI gate locally**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --locked && cargo check --locked`
Expected: all clean (mirrors `ci.yml`).

- [ ] **Step 6: Commit and report**

Run: `git add README.md && git commit -m "docs: update registry counts for phase-1 lanes"`
Then present the execution report: lanes shipped/dropped with per-lane smoke evidence, sweep findings, catalog changes, full-harvest exit code and record counts.

---

## Self-Review

**1. Spec coverage:**
- Goal (four axes) → Tasks 1-5 (volume/coverage), 6-7 (freshness/resilience via sweep).
- Plain-GET/keyless/no-new-deps/no-contract-change constraints → Global Constraints; no task touches `model.rs`/`normalize.rs`/`fetch.rs`/`runner.rs`/shared `parse.rs`.
- Wave 1 lane table: us-proxy → Task 1; proxyscrape https → Task 2; spys → Task 3; Scrappey → Task 4; roosterkid → Task 5. ✅
- Verified exclusions (hproxy/89ip/proxifly/anonymous) → no task adds them; Review Focus #1-5 pin the probe conditions. ✅
- Wave 2 (re-verify + discovery + wire) → Tasks 6-7. ✅
- Failure semantics → drop-on-failure rules in Tasks 1-7; Task 8 Step 1 contract check.
- Testing section → unit tests per task; registration smoke (Task 4 Step 8, Task 8 Step 2); live smoke per lane; CI gates (Task 8 Step 5).
- Acceptance 1-5 → Task 8 Steps 1-5. ✅
- Gap found and fixed: acceptance criterion 3 (catalog dated today) is Task 6; criterion 5 (README counts) is Task 8 Step 4. No gaps remain.

**2. Step scan:** each step = one probe (command + expected), one test (named, with exact assertions), one code change (exact location + snippet), one run (command + expected), or one commit (exact subject). Probes carry explicit drop conditions, and four fixture decisions are deferred to probe evidence rather than guessed: Scrappey's row structure (table vs SSR JSON), spys surviving mirror URLs, the us-proxy textarea presence, and the proxyscrape `ssl` subset semantics. Every drop rule explicitly names which steps to skip so no failing test or half-wired lane survives a dropped lane.

**3. Type consistency:** `requests() -> Vec<Request>`, `parse_rows(body, Option<Scheme>) -> Vec<ProxyRecord>`, `PROTOCOLS: [(Scheme, &str, &str); 4]` (3-tuple of scheme/label/query-fragment after the Task 2 restructure), `gh!` macro arg order — all match the inspected definitions; `protocols()` string updated alongside `PROTOCOLS` in Task 2 (display contract). Evidence commands use the verified artifact schema: `<stem>.provenance.jsonl` with `source` field, `<stem>.summary.json` with `outcome`/`exit_code`/`providers[].records`.

**4. Review Focus:** all five lines have their owning task and test: #1 → Task 2 Steps 1-2; #2 → Task 1 Steps 1-2 + smoke Step 7; #3 → Task 3 Steps 1-2; #4 → Task 4 Steps 1-2 + smoke Step 9; #5 → Task 5 Step 1.

**5. Proportion:** ~700 lines for 9 tasks covering baselines, probes, 4 lane wirings, a sweep, and acceptance — code blocks are fixtures/snippets, not bodies; `parse_rows`' body and the spys/roosterkid wiring are deliberately deferred to probe output (spec forbids guessing source quirks).

**6. Post-oracle fixes applied (all 8 findings + Scrappey advisory):** Task 2 probe rewritten from unsatisfiable `protocol=https` distinctness to `ssl=true` subset semantics; evidence commands moved to verified artifact schema (finding 2-4); drop rules now name skipped steps (finding 5); Task 0 baseline added (finding 6); roosterkid branch probed via `ls-remote` (finding 7); `mod.rs` doc counts in Task 4 Step 6 (finding 8); Scrappey structure verified live with `data-protocol` per-row scheme and SSR-JSON fallback (advisory).
