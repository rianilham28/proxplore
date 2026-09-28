# Phase 1: HTTP/SOCKS Source Expansion — Design

Date: 2026-09-28
Status: design approved in chat; awaiting user review of this spec (then
writing-plans)
Sub-project 1 of the proxplore extension series (source expansion → headless
transport → node formats → output/CLI contract, each its own spec).

## Goal

Grow the harvested HTTP/HTTPS/SOCKS4/SOCKS5 pool along four axes — raw pool
size, freshness, coverage, and resilience — by wiring remaining keyless
plain-GET lanes from `free-proxy-sources.md` plus a re-verify/discovery sweep.
Steady-state success: each shipped lane contributes unique post-dedupe records
in a live smoke run.

## Constraints (hard boundaries)

- **Plain-GET only.** Every lane must run through the existing
  `Provider`/`ParseKind`/`fetch.rs` stack with no changes to those contracts.
- **Keyless-only charter holds.** No tokens, no signup-derived endpoints
  (Webshare excluded). Node formats, gated feeds, and credential pools are
  out of phase 1.
- **Consumer contract untouched.** `model.rs`, `normalize.rs`, `fetch.rs`,
  `runner.rs`, shared `parse.rs`, `.jsonl`/`.summary.json` schemas, and exit
  codes (0/1/2/130) are byte-for-byte behaviorally unchanged — proxalyze's
  nightly consumer depends on them.
- **No new dependencies.** Parsers use in-tree `regex`/string handling.
- **No product code before spec approval** (brainstorming gate).

## Approach

Incremental lane wiring (wave 1) with a parallel re-verify + discovery sweep
(wave 2). The runner's loud-failure semantics (dead source → partial data,
exit 2) make stale-catalog wiring safe: failures are observable at smoke time,
never silent.

## Wave 1 — sub-lane deepening

| Lane | Change | File(s) | Condition |
|---|---|---|---|
| free-proxy-list `us-proxy.html` | +1 `Request` row; reuse textarea parser if the page embeds the raw list (probe) | `free_proxy_list.rs` | probe confirms parseable rows |
| proxyscrape `https` lane | Add `(Scheme::Https, …)` to the 3-entry `PROTOCOLS` table with correct param spelling | `proxyscrape.rs` | live probe confirms distinct rows exist |
| spys sub-lists (socks/https) | Add `Request` rows against `spys.me` mirrors | `spys.rs` | probe confirms mirrors exist sans JS |
| Scrappey tool page | New single module: one `Request`, provider-local `Custom` table parser | `providers/scrappey.rs` (new), `mod.rs` | parser fixture tests + smoke >0 rows |
| roosterkid GitHub feed | New `gh!` row | `github_feeds.rs` | re-verification shows live rows (default: skip) |

### Verified exclusions (inspected code, zero new volume)

- hproxy `all.txt` = union of the four wired protocol files;
  `live.txt`/by-country/API = subsets of wired lanes.
- 89ip `/api.html` = same 4,480-IP pool as the ~110 already-seeded pages.
- proxifly country lanes = partitions of the wired protocol files.
- free-proxy-list `anonymous-proxy.html` = the already-wired main page.
- roosterkid's catalog size is 6 decayed rows — wired only if live.

A lane that fails its probe or smoke test is **dropped, not force-fitted**.

## Wave 2 — re-verify + discovery sweep (parallel with wave 1)

1. Re-fetch every `free-proxy-sources.md` row's entry point; update Status
   columns (live/reported/blocked/dead) with today's evidence.
2. Research sweep for new keyless HTTP/SOCKS sources not yet cataloged.
3. Wire live plain-GET findings as additional request rows or modules,
   same rules as wave 1 (probes, smoke, drop-on-failure).

## Failure semantics

Inherited unchanged: dead endpoint → `requests_ok/total` mismatch + warning +
exit 2 with partial data kept; 429 → retry ladder then failure; per-host
blackhole → circuit breaker. A 200-response lane parsing to 0 records is a
smoke-test defect → fix or drop before merge.

## Testing

- Unit: fixture-based parser tests for Scrappey rows and us-proxy textarea
  (pattern: `e89ip.rs`); proxyscrape URL-construction assertions (pattern:
  `geonode.rs`).
- Registration: `guard_registry_integrity` + `--list-providers` smoke.
- Live smoke (throwaway, evidence pasted in the implementation PR):
  `cargo run --release -- --providers <id> --output /tmp/<id>.txt` per lane.
- CI gates: fmt, clippy `-D warnings`, tests, MSRV `cargo check --locked`,
  cargo-audit.

## Acceptance criteria

1. Every shipped lane yields >0 unique post-dedupe records in a live smoke
   run, or is removed.
2. Full default harvest exit codes and `.jsonl`/`.summary.json` output
   remain byte-compatible with proxalyze's reader.
3. `free-proxy-sources.md` statuses reflect verification as of today.
4. CI green on all existing gates.
5. README provider/lane counts updated to match the registry.

## Out of scope (later specs)

Headless transport for blocked sites (kuaidaili/hide.mn/freeproxylists/
proxy-listen), `Scheme`/normalize changes, node-subscription formats,
gated/token feeds, output/CLI contract changes.
