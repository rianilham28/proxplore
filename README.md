# proxplore

Harvest free HTTP/HTTPS/SOCKS4/SOCKS5 proxies from keyless public sources
into a deduplicated `proxies.txt` — one `scheme://[user:pass@]host:port` per
line. proxplore **only harvests**: reachability checking, judging and scoring
are the job of [proxalyze](https://github.com/rianilham28/proxalyze), its
sibling and downstream consumer.

## Scope

30 identified providers across the catalog's three keyless lanes
(`free-proxy-sources.md` documents the research behind every row):

- **TXT/JSON/CSV APIs** — proxyscrape, geonode, databay, pubproxy, proxifly,
  proxylister
- **HTML list pages** — proxydb, free-proxy-list.net, spys (TXT mirror),
  advanced.name, 89ip, ip3366
- **GitHub raw feeds** — thespeedx, hproxy, monosans, databay-labs, xyzs996,
  aliilapro, vpslabcloud, iplocate, fate0, sunny9577, hideip-me, hookzof,
  vakhov, rix4uni, blitzproxy, proxio-io, m1noa-proxypool, proxygenerator1

Deliberately excluded: node-subscription formats (vmess/vless/trojan/ss),
account-gated grants, and credential-leak "free socks5" pools.

## Usage

```bash
cargo run --release                         # every provider -> proxies.txt
cargo run --release -- --list-providers     # the registry
cargo run --release -- --providers geonode,proxydb --output /tmp/one.txt
```

- Fetch width is sized from the machine (cpus, free memory, fd limits — the
  soft `RLIMIT_NOFILE` is raised toward hard at startup); `--concurrency`
  overrides.
- Paged sources (geonode/databay/proxylister) follow envelope totals;
  depth-bounded listings (proxydb/89ip) fan out every page in parallel.
  Every source is harvested to its end, or truncates **loudly** with partial
  data kept — `requests ok/total` plus a warning line tell you which.
- Deep feeds rate-limit single IPs (429 / SYN-drop). A rotating gateway
  removes the wall: `--proxy-url http://user:pass@gw:1338`, or set
  `PROXPLORE_PROXY_URL` to keep the secret off argv. Without one, expect
  proxydb/89ip/databay to truncate politely.

## Design

`src/model.rs` is the contract: a provider is an identified module declaring
`requests()` (seed plan) and optionally `advance()` (pagination to
exhaustion). Parsing dispatches through `ParseKind` — shared components
(`entries`/`json`/`ndjson`/`auto`) or a provider-local `fn` for one-source
quirks (proxydb's decoy-port hrefs, advanced.name's per-row base64 attrs,
89ip's layui cell pairing). Everything funnels through `normalize.rs`, whose
non-routable IP filter is a bit-exact port of CPython's `ipaddress` tables
(pinned by differential tests). Transport is
[wreq](https://crates.io/crates/wreq) with Chrome TLS/H2 emulation — the
Rust counterpart of curl_cffi's `impersonate`.

## CI / consumer contract

- `ci.yml`: fmt + clippy `-D warnings` + `cargo test` + release-build,
  ubuntu/macos.
- `release.yml`: tag `v*` → linux-gnu x86_64/aarch64 + separate macOS
  x86_64/aarch64 archives, sha256'd, attached to the GitHub Release.
- proxalyze's nightly pool refresh checks this repo out and runs
  `cargo build --release --locked --bin proxplore` — keep `Cargo.lock`
  committed and the binary name stable.
