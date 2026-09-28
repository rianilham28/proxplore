# Free Proxy Source Catalog — 2026-09-28

Six-lane research sweep; every non-"reported" entry was fetch-verified. §1/§2/§3 re-fetched end-to-end on 2026-09-28 (§10); a second 2026-09-28 wave added 4 new §3 feeds and recorded its rejections in §3a; §4–§6 last verified 2026-09-11/12.
Verification legend: **live** = fetched and saw real data · **reported** = first-party docs/pages only · **blocked** = site up, scrapers refused · **dead** = gone (NXDOMAIN, 4xx/5xx, parked, or archived).

---

## 1. Keyless HTTP APIs (TXT/JSON/CSV, no account)

| Provider | Entry point | Format | Protocols | Cadence | Notes |
|---|---|---|---|---|---|
| **ProxyScrape Public API** | `https://api.proxyscrape.com/v4/free-proxy-list/get?request=display_proxies&protocol=http&proxy_format=ipport&format=text` | TXT/JSON/CSV | HTTP, HTTPS, SOCKS4/5 | ~1 min | Params: `protocol, timeout(ms), anonymity(elite/anonymous/transparent), country=ISO, ssl, limit, skip` (nextpage flag); legacy v2 endpoint still alive. Use `protocol=`, `proxytype` is silently ignored. **`ssl` is a yes/no boolean, not `true`/`false` — `ssl=true` returns HTTP 400 `Invalid ssl, please use yes or no.`; `ssl=yes` (the https lane) returns 200.** The ssl-filtered slice is NOT a subset of the unfiltered `protocol=http` pool — cited figures 1,287 vs 501 unique with 29 exclusive rows; re-measured 2026-09-28 across two back-to-back fetches at 1,635/475 and 1,527/475 unique, with 172 and 152 entries exclusive to the ssl lane. The endpoint serves a fresh snapshot per request (`cache-control: no-store`, `cf-cache-status: EXPIRED`/`HIT`), so exact counts move between calls while the non-subset property is stable. 1.2–3k HTTP entries. **live** (2026-09-28) |
| **GeoNode** | `https://proxylist.geonode.com/api/proxy-list?limit=20&page=1&sort_by=lastChecked&sort_type=desc&protocols=http` | JSON (geo, ASN, latency, uptime, lastChecked) | HTTP, HTTPS, SOCKS4/5 | continuous | **Requires browser-like User-Agent (403 otherwise).** `sort_by`+`sort_type` only — `sort_order` → 400. Filters: `protocols, anonymityLevel, countries(CSV), upTime, latency, google=true`. ~2.7k HTTP. **live** (2026-09-28: total=950, fresh lastChecked) |
| **Databay API** | `https://databay.com/api/v1/proxy-list?protocol=http&limit=500&format=json` | JSON + TXT/CSV | HTTP(S), SOCKS4/5 | 5 min | TXT hotlinks: `databay.com/free-proxy-list/{http,https,socks4,socks5}.txt` (strip `#` lines). `ssl=strict` matches their GitHub pool. Max 1000/page. **live** (2026-09-28: total=6,048) |
| **PubProxy** | `http://pubproxy.com/api/proxy?limit=20&format=txt` | TXT/JSON | HTTP, SOCKS4/5 | frequent scans | Small pool (~95 alive), one-random-proxy default; optional free key raises caps. Site is HTTP-only. **live** (2026-09-28) |
| **Proxifly** | `https://cdn.jsdelivr.net/gh/proxifly/free-proxy-list@main/proxies/protocols/http/data.txt` | TXT/JSON/CSV (scheme-prefixed lines — parse after `://`) | HTTP(S), SOCKS4/5 | 5 min | Path template `.../proxies/protocols/{proto}/data.{ext}` and `.../proxies/countries/{ISO}/data.txt`; swap `@main` for commit hash to bust CDN cache. Hosted keyless REST: POST `api.proxifly.dev/get-proxy` (1–20/call, rate-limited; 0% measured live rate in 2026-07 benchmark). **live** (CDN feed; 5,306 lines 2026-09-28) |

## 2. HTML list pages (scrape the page; several embed raw TXT blocks)

| Provider | Entry point | Protocols | Volume/cadence | Scraping notes | Status |
|---|---|---|---|---|---|
| **Free-Proxy-List.net** (Scraped Ninja) | `https://free-proxy-list.net/` | HTTP/SOCKS mixed | ~300 rows, every 10 min | Plain GET works, no anti-bot. Each page embeds a "Raw Proxy List" ip:port textarea — scrape that. Sub-pages: `/socks-proxy.html` (legacy socks-proxy.net), `/ssl-proxy.html` (legacy sslproxies.org), `/us-proxy.html` (legacy us-proxy.org), `/anonymous-proxy.html`. ⚠️ Old country subdomains (us./socks5./scoped-v3.) are NXDOMAIN — scheme is dead. | **live** (2026-09-28: 303 textarea rows) |
| **ProxyDB.net** | `https://proxydb.net/` | HTTP(S), SOCKS4/5 | 6,394 browsable, continuous re-check | Pagination `?offset=N` step 30; sort `?sort_column_id=ip|port|protocol|country|anonlvl|uptime|response_time_avg|checked&sort_order_desc=true`. Rich columns (leaked-header evidence, gateway IP, GeoIP). **Server-rendered to plain GET — verified: 31 `<tr>`/page, no challenge, no JS required.** ⚠️ Extract ip:port from each row's `<a href="/IP/PORT#proto">` link, NOT cell text — port cells carry an invisible decoy digit (`<div style="display:none">12</div><a>80</a>` → "1280") that silently corrupts naive scrapes. | **unverified — connection timeout from sweep host** (2026-09-28: DNS resolves to 85.214.115.35, but TCP connect to 443 *and* 80 times out across 5 curl attempts + 4 raw socket attempts, no handshake. A connection-level failure — **not** the 403/503 scraper-wall signature and **not** proof of death; the 2026-09-12 log had it serving 32 rows/page. Re-probe from a second vantage point before scheduling. Cross-referenced in §8, deliberately NOT listed as dead.) |
| **SPYS.one** | `https://www.spys.one/en/` | HTTP(S), SOCKS4/5 | 32,612 rolling pool, near-realtime | ⚠️ Ports obfuscated for non-JS clients (inline-JS computed). Use headless JS **or the operator's TXT mirror: `http://spys.me/proxy.txt` (live)**. Sub-pages: `/en/socks-proxy-list/`, `/en/https-ssl-proxy/`, `/en/anonymous-proxy-list/`, per-country `/free-proxy-list/<CC>/`; `?page=N`. | **live** (2026-09-28: HTML 301 → `spys.one/en/`; TXT mirror 200, 400 rows) |
| **advanced.name** | `https://advanced.name/freeproxy` | HTTP(S), SOCKS4/5 | ~433 live, minutes | Cells empty in text but server-side base64 in `<td data-ip="…" data-port="…">` — b64-decode attributes, no JS needed. Filters `?type=socks5|http|anon|elite…`, `?country=<cc>`. No challenge. | **live** (2026-09-28: 100 `data-ip=` attrs) |
| **89ip.cn** (CN) | `https://www.89ip.cn/` | HTTP/HTTPS (+daily SOCKS5 claimed) | 4,480 IPs, multiple/day | Pagination `/index_N.html`; `/api.html` has free API endpoints; rows carry CN province/city/carrier — good for CN-exit, low foreign reachability. | **live** (2026-09-28: 41 rows/page) |
| **ip3366.net** (CN) | `http://www.ip3366.net/free/` | HTTP/HTTPS | 100 rows, ~24 h | **Use http:// — TLS chain broken.** Pagination `/free/?stype=1&page=2`. 15 rows/page ×7; showcase-grade. Same operator family as 89ip. | **live (degraded)** (2026-09-28: 200, gb2312-encoded, 16 `<tr>` = 15 data rows; ip and port sit in **separate table cells** so a naive `ip:port` regex finds 0 hits — concatenate cells; last-verified dates 2026/9/3–9/4, ~1 week stale) |
| **Scrappey tool page** | `https://scrappey.com/tools/free-proxy-lists/socks5` | SOCKS5/4, HTTP(S) | 180 total, ~15 min | Server-rendered rows, single page, client-side tab filters. Filler only — many rows self-flagged "likely dead". | **live (low quality)** (2026-09-28: 180 `data-protocol=` rows, 60/60/60 http/socks4/socks5 — scheme lives per row, never assume http) |
| **KuaiDaiLi** (CN) | `https://www.kuaidaili.com/free/inna/` | HTTP(S) | rolling | Tencent EdgeOne JS challenge (`__tst_status`/EO_Bot_Ssid cookie then reload) — headless browser only. Patterns (reported): `/free/inna/N/` CN high-anon, `/free/inwa/N/` overseas. | **blocked** (2026-09-28: 200 but 983-byte challenge stub, 0 rows) |
| **hide.mn / hidemy.name** (RU) | `https://hide.mn/en/proxy-list/?proxy=ipv6` | HTTP(S), SOCKS4/5, **IPv6 filter** | near-minute re-check | 403 to any non-browser client; exports paywalled ~$5. Params: `country=, type[], level[], start=` (64/page). Browser-context scraper only. | **blocked** (2026-09-28: 403, 5,339-byte block page) |
| **freeproxylists.net** | `https://www.freeproxylists.net/?pr=SOCKS5` | SOCKS5/4, HTTP | continuous | 403 UA/TLS-fingerprint wall — Playwright + real Chrome UA. Params: `?pr, ?c=<country>, ?page=N, ?sg`. | **blocked** (2026-09-28: 403, 15-byte refusal) |
| **proxy-listen.de** | `https://www.proxy-listen.de/Proxy/Proxylist?lversion=6` | HTTP/SOCKS, rare **IPv6 filter** | continuous (reported) | Expired TLS cert (fetch fails without insecure flag). Params: `lversion=6, socks=1, lanon, country`. | **blocked (cert + 503)** (2026-09-28: TLS `certificate has expired (10)`; insecure retry → 503, 0 rows — worse than cert-only) |
| **ProxyScrape HTML page** | `https://proxyscrape.com/free-proxy-list` | all | 1 min | SPA — static HTML has zero rows; use the §1 API instead. | live (not scrapable as HTML; 2026-09-28: 274 KB shell, 0 data rows) |

## 3. GitHub raw ip:port feeds (cron-refreshed, no key)

All entry points are `https://raw.githubusercontent.com/...` — no UA, no auth. Cross-feed overlap is heavy; dedupe by ip:port.

| Feed | Entry point | Size / cadence | Notes | Status |
|---|---|---|---|---|
| **TheSpeedX/PROXY-List** | `.../TheSpeedX/PROXY-List/master/http.txt` | ~2k socks5 verified; daily | Siblings `socks4.txt, socks5.txt, openapi.txt` (type-annotated). Oldest still-alive feed. Dense IN/BD/SEA 4145/1080 relays. | **live** (2026-09-28: 2,681 rows) |
| **hproxy-com/free-proxy-list** | `.../hproxy-com/free-proxy-list/main/http.txt` | 4,327 lines; several×/day | Richest repo: `https.txt socks4.txt socks5.txt all.txt live.txt elite.txt fast.txt`, `all.json/live.json/all.csv/live.csv`, `by-country/<CC>.txt`. Keyless API at hproxy.com. | **live** (2026-09-28: 18,997 rows — repo has grown several×) |
| **monosans/proxy-list** | `.../monosans/proxy-list/main/proxies/http.txt` | few hundred, high-uptime; hourly re-check | Published output of monosans/proxy-scraper-checker. `proxies.json` = best machine-readable niche feed: protocol, username/password, timeout, exit_ip, ASN, GeoLite2 geo. TXT lines may carry `user:pass@host:port`. `all.txt` prefixes `socks5://` — drops into requests proxies dict. | **live** (2026-09-28: 329 rows) |
| **databay-labs/free-proxy-list** | `.../databay-labs/free-proxy-list/master/http.txt` | ~2,946; 5 min claimed | ⚠️ default branch `master`. TLS-validated subsets; `by-country/`. | **live** (2026-09-28: 3,480 rows) |
| **xyzs996/free-proxy-health-list** | `.../xyzs996/free-proxy-health-list/main/socks5.txt` | 519 socks5; rechecked 30 min | `http.txt https.txt socks4.txt all.txt` + `proxies/`, `stats/` per-country health JSON. Pages mirror: `xyzs996.github.io/free-proxy-health-list/`. | **live** (2026-09-28: 543 socks5 rows) |
| **ALIILAPRO/Proxy** | `.../ALIILAPRO/Proxy/main/http.txt` | ~600; hourly | Small but very fresh. `socks4.txt, socks5.txt`. | **live** (2026-09-28: 990 rows) |
| **VPSLabCloud/VPSLab-Free-Proxy-List** | `.../VPSLabCloud/VPSLab-Free-Proxy-List/main/all_proxies.txt` | ~1,424; several×/day | Anonymity×SSL matrix: `http_elite, http_ssl, http_anonymous, socks4_all, socks5_all, all_ssl_elite…`. ⚠️ strip `#` header lines. | **live** (2026-09-28: 2,003 rows) |
| **iplocate/free-proxy-list** | `.../iplocate/free-proxy-list/main/protocols/http.txt` | ~577–1.2k; 30 min | Best measured live-rate of its class (11.7%, 2026-07 benchmark). `protocols/{http,https,socks4,socks5}.txt`, `countries/US/proxies.txt`. | **live** (2026-09-28: 574 rows) |
| **fate0/proxylist** | `.../fate0/proxylist/master/proxy.list` | thousands; continuous | NDJSON per line (host, port, type, anonymity, country, response_time). Stream-parse; HTTP(S) only. Distinct from dead fate0/getproxy. | **live** (2026-09-28: 251 NDJSON rows — pool has shrunk well below "thousands") |
| **vakhov/fresh-proxy-list** | `.../vakhov/fresh-proxy-list/master/proxylist.txt` | 728; hours | `http.txt`, `socks*.txt` siblings. Validate before use (0% rate once measured). | **live** (2026-09-28: 722 rows) |
| **sunny9577/proxy-scraper** | `.../sunny9577/proxy-scraper/master/generated/http_proxies.txt` | 2,003; scheduled | Go scraper's generated output; socks siblings exist in repo layout (unverified this sweep). | **live** (2026-09-28: 1,957 rows) |
| **zloi-user/hideip.me** | `.../zloi-user/hideip.me/main/http.txt` | 130+; frequent | Format `ip:port:COUNTRY_FULL` — 3 fields, full country names not ISO. | **live** (2026-09-28: 194 rows) |
| **hookzof/socks5_list** | `.../hookzof/socks5_list/master/proxy.txt` | ~180–250; CI-refreshed | Protocol-pure SOCKS5 (RU/IR/VN/ID/BD heavy; Telegram-compatible lane). No creds baked in. | **live** (2026-09-28: **27,050** rows — the "~180–250" figure was two orders of magnitude stale) |
| **roosterkid/openproxylist** | `.../roosterkid/openproxylist/main/SOCKS4_RAW.txt` + `.../main/HTTPS_RAW.txt` | ~149 socks4 / ~59 https rows ⚠️ | Branch `main`. Both wired lanes are **live**; `SOCKS5_RAW.txt` decayed to 4 rows on the 2026-09-28 re-probe and is **intentionally left unwired** (below the ≥10-row gate), as are `HTTP_RAW`/`ALL_PROXIES_RAW` (404). SOCKS5 lane marginal — prefer hookzof/monosans. | **live** (2026-09-28: SOCKS4_RAW 149 rows, HTTPS_RAW 59 rows — both wired; SOCKS5_RAW 4 rows, unwired) |
| **ErcinDedeoglu/proxies** | `.../ErcinDedeoglu/proxies/main/proxies/http.txt` | 64,902 http / 48,230 socks5; hourly | Branch `main`; files nest under `proxies/`. Largest hourly pool found in the 2026-09-28 wave (http 64.9k, https 3.2k, socks4 22.2k, socks5 48.2k distinct tokens). Wired as `ercindedeguoglu`. | **live** (2026-09-28: 138,457 output lines) |
| **TuanMinPay/live-proxy** | `.../TuanMinPay/live-proxy/master/http.txt` | 8,556 http; hourly | `http.txt socks4.txt socks5.txt all.txt`, CI auto-update workflow. **No `https.txt` — that path 404s**, so the lane ships three protocol files, not four. Wired as `tuanminpay`. | **live** (2026-09-28: 44,409 output lines) |
| **Zaeem20/FREE_PROXIES_LIST** | `.../Zaeem20/FREE_PROXIES_LIST/master/http.txt` | 191 http; hourly | Branch `master`. Cleanest small pool of the wave (http 191, https 576, socks4 86, socks5 333). Wired as `zaeem20`. | **live** (2026-09-28: 1,186 output lines) |
| **mmpx12/proxy-list** | `.../mmpx12/proxy-list/master/http.txt` | 440 http; daily | `http.txt https.txt socks4.txt socks5.txt` + `tor-exit-nodes.txt` and per-IP `.ovpn` files (VPN out of scope). ⚠️ `http.txt` currently leads with a literal `error code: 502` line — the parser drops it, but the upstream occasionally serves an error body with 200. Wired as `mmpx12`. | **live** (2026-09-28: 1,591 output lines) |

### 3a. Wave-2 discovery — rejected candidates (2026-09-28)

| Candidate | Evidence | Verdict |
|---|---|---|
| sockslist.us (`/proxiesraw/*.txt`, `/api/v1/proxy`) | 404 on all four probed paths, 24.7 KB error shell, 0 tokens | **dead** — the API documented in third-party writeups no longer resolves |
| OfficialPenguin/PROXY-List, MuRongPIG/Proxy-List, caliphdev/Proxy-List, UserR3X/proxy-list, yemixzy/proxy-list, ZEROSEB/proxylist-update-every-minute, 89trillion/free-proxy, clxrc/Proxy-List | `git ls-remote` returns no HEAD | **dead** — repos gone |
| ShiftyTR/Proxy-List | 200 with 40 http / 661 socks4 tokens, but last commit to `http.txt` 2023-08-11 | **dead (stale)** — ≥3 years without refresh; size alone doesn't qualify |
| proxy4parsing/proxy-list | 200 with 19,031 http tokens, last commit 2024-04-27 | **dead (stale)** |
| clarketm/proxy-list | 400 lines in `proxy-list-raw.txt`, `last-status-update.txt` reads `Tue Mar 21 23:33:19 PDT 2023` | **dead (stale)** |

## 4. Node-subscription feeds (vmess/vless/trojan/ss/hysteria2/tuic — Clash Verge/mihomo/NekoBox/v2rayNG)

| Feed | Entry point | Types | Cadence | Notes | Status |
|---|---|---|---|---|---|
| **0xRadikal/Free-v2ray-Configs** | `.../0xRadikal/Free-v2ray-Configs/main/all/configs.txt` | vless, vmess, trojan, ss, hy2, tuic, anytls, wireguard | every 15 min | ~11.6k configs; `configs_base64.txt` (subscription import), `clash.yaml` (4.5 MB), `singbox.json`, `Countries/` per-CC. `#profile-update-interval: 1`. Range-request big files or jsdelivr. | **live** |
| **LeilaoMi/AutoMergePublicNodes-Optimized** | `.../LeilaoMi/AutoMergePublicNodes-Optimized/main/output/global.urls` | vless, vmess, trojan, ss, hy2 | several×/day | sing-box real-tested "verified" tier, latency in node names. `global.txt` (base64 sub), `global.yaml` (clash), `all.*` full lists, `by_protocol/`, `by_region/`, `chunks/`, `health_report.json`. | **live** |
| **mfuu/FreeProxies** | `.../mfuu/FreeProxies/master/sub.yaml` | vmess, trojan, ss, vless | several×/day | Clash `proxies:` list 900+ lines; `sub` = base64 subscription; `list.json` = upstream sub URLs. ⚠️ many trojan relays share password `humanity` (filler) — latency-filter. | **live** |
| **xyfqzy/free-nodes** | `.../xyfqzy/free-nodes/main/ppt.txt` | ss/authenticated (host:port:user:pass) | every 2 h | 11,019 credential rows (sspanel-style). Also Base64/YAML clash/mihomo/v2rayN/Shadowrocket outputs, Pages site nodes.udptoos.com. Source of auth-required endpoints, not open relays. | **live** |
| **zhuhaiuk/free-nodes** | `.../zhuhaiuk/free-nodes/main/nodes.txt` | https-proxy, trojan, vless(reality), anytls | hourly | Single-line base64 → decode; ~25 curated clean nodes. `clash_config.yaml` mihomo-ready, `extra_nodes.txt`, `data/`, `archive/`. Small but freshest. | **live** |
| **morpheusadam/v2ray-config** | `.../morpheusadam/v2ray-config/main/proxies/all.txt` | http/socks5 lines + vless/vmess/trojan/ss/hy2/tuic/reality | daily | IR/censorship-circumvention oriented; format `scheme://host:port | CC | latency | age` — split on ` | `, token 1. Publishes per-run proof density (23% measured 2026-09-11: "1971 entries, 794 proved"). ⚠️ famous IR repo Mrc0113/socks5 was deleted. | **live** |

## 5. Account-signup free grants (the "Webshare gives 10" category)

### Genuinely free (no card)

| Provider | What you get | Auth | Access | Status |
|---|---|---|---|---|
| **Webshare** | **10 datacenter proxies + 1 GB/mo, permanent**, no expiry, no CC (shared pool with other free users) | signup | `proxy.webshare.io/api/v2/proxy/list/?mode=direct` — header `Authorization: Token <key>` (**Token, not Bearer**). CSV/TXT export in dashboard. | **live** |
| **Oxylabs Free Datacenter plan** | 5 US datacenter IPs, 5 GB/mo shared, 20 concurrent, 1-month validity, no CC | signup | `dc.oxylabs.io:8001–8005`, user-pass `user-{acct}-session-{id}` | **live** |
| **Bright Data** | **5,000 credits/mo (~$7.50) recurring free** on Unlocker/SERP/Scraper APIs (no card); proxies NOT in free tier: one-time **$2 trial credit (7 d)**; +$5 after adding payment method | signup (+KYC for resi/mobile) | zones via `brd.superproxy.io:33335`; prepaid wallet hard-stops at $0 | **live** |
| **GeoNode** | 1,500 scraping-API req/mo, permanent, no card | signup | app.geonode.com key; $0.13/1K after | **live** |
| **RoundProxies** | 500 MB residential, 24 h, no CC (email verify + use-case) | signup | rotating res, country/city targeting | reported |
| **Byteful** (+ twin PingProxies) | **1 GB residential, no time limit, no CC — but KYC-gated** (selfie/ID) | signup | user-pass suffixes, dashboard+API | reported |
| **AceProxies** | 1 dedicated datacenter/SOCKS proxy, 24 h, one-time, no CC (manual form) | signup+email-confirm | panel delivery | **live** |
| **QuarkIP** | 200 MB residential trial, no CC | signup | dashboard-generated only, no API | reported |
| **MoMoProxy** | 1 GB residential trial, no CC, issued manually via support chat | signup | host/port/user/pass from support | reported |
| **LunaProxy** | claimed 1 GB residential trial no CC | signup | unconfirmed from official site this session | reported (low conf.) |
| **LimeProxies** | trial advertised, no CC; size unpublished | signup | verify at signup | reported (low conf.) |
| **FloppyData** | "free trial" first-party (7-day res reported), size unconfirmed | signup | dashboard + `api.floppydata.net` X-Api-Key | **live** (quota TBD) |
| **Massive (joinmassive)** | sign-up trial bandwidth; **Massive for Startups: 1 TB/mo free ×3 months, no card/equity** | signup | `network.joinmassive.com:65535` HTTPS / `:65533` SOCKS5, `--proxy-user 'USER:API_KEY'` | reported |
| **PacketStream** | discretionary "trial credits" via request (manual); earn-side $0.10/GB bandwidth sharing | signup | per-country gateway ports, HTTP/SOCKS5 | **live** (manual) |

### Trials / paid-gate (card or money required — included for completeness)

| Provider | Terms |
|---|---|
| **Decodo** (ex-Smartproxy) | 3-day trial, 100 MB res; card/PayPal/GPay mandatory, auto-activates paid plan day 3. Separate $1/1-yr Web Scraping API credit. **live** |
| **SOAX** | $1.99 → 3 days + 400 MB across all types (paid trial, no auto-convert). reported |
| **ProxyEmpire** | explicitly no free trial; $1.97 → 100 MB res + 50 MB mobile. **live** |
| **NodeMaven** | no free; $3.50 → 750 MB res (official; third-party "1 GB free" claims are false). **live** |
| **IPRoyal** | no individual trial at all; enterprise-only trial via sales. Free HTML list page only. **live** |
| **DataImpulse** | explicitly no free trial; $5 intro 5 GB PAYG. reported (negative entry) |
| **Proxy-Seller** | FAQ: no free trial; 24 h refund window = pseudo-trial; IPv6 from $0.02/IP. **live** |
| **Zyte API** | $5 signup credit, no card. **live** |
| **ScrapingBee** | 1,000 credits one-off, no card. **live** |
| **ScraperAPI** | 1,000 cr/mo permanent free; 5,000 first 7 d. **live** |
| **ScrapeOps** | 1,000 cr/mo permanent, 1 concurrent, no card. **live** |
| **Zenrows** | 5,000 cr/mo permanent, no card (premium-proxy req = 10 cr). **live** |
| **Oxylabs WSA** | 2,000 free results trial, no expiry; Unblocker 1 GB trial no CC. **live** |
| **Decodo WSA** | ~2,000 std req/mo at $0. **live** |
| **Crawlbase** | 1,000–20,000 free req (promo-dependent), no card. reported |
| **Scrappey** | free demo trial, no card, then PAYG €0.10/1k. reported |
| **SerpApi** | 250 searches/mo, no card. **live** |
| **ScrapeStack** | 100 req/mo. **live** |

### Dead signup sources (purge)
**GeoSurf** (wound down into Decodo; site unreachable) · **NetNut** (FBI seizure + pool disruption 2026-07-02 — treat NetNut-sourced pools as suspect) · **Egress** (domain now KnowBe4 email-security, no proxy product) · **Virtuino** ("free 5 daily proxies" claim unsubstantiable; virtuino.net dead) · **proxy66.cloud/net/org** (all NXDOMAIN) · **Cyberocean, QuarkGrid, HighProxy, GrabALoop, ProxyUptime, Proxying.io, ABCProxy free tier** (gone/no free).

## 6. Developer free-tier APIs (fetch-through & proxy gateways)

Full table data is in §5's trial block plus: **Webshare API** (also keyless download endpoint `proxy.webshare.io/api/v2/proxy/list/download/{token}/{cc}/any/{username|sourceip}/{direct|backbone}/{search}/` → plain-text `ip:port:user:pass`), **ProxyScrape Account API** (`/v4/account/...`, header `api-token`), **Zyte** proxy-mode (`curl -k -x https://api.zyte.com:8011 -U 'KEY:'`), **Oxylabs Unblocker** backconnect (`unblock.oxylabs.io:60000`), **Decodo Unblocker** (`unblock.decodo.com:60000`), **Crawlbase** proxy-mode (`smartproxy.crawlbase.com:8012/8013`), **Massive** gateway ports above, **ScrapingBee** (`premium_proxy=true`), **ScraperAPI** (`premium=true` residential allowed on free plan), **Zenrows** (`mode=auto` adaptive billing), **ScrapeOps** (`provider=`, `session=`, only successful responses billed).

## 7. Caveats for a scraper pipeline

1. **Dedupe** — heavy cross-feed overlap (45.74.31.x Hetzner pool, 103.x/116.x relay blocks appear in 3+ feeds).
2. **Liveness** — open-relay feeds measure 0–12% live rates; pre-filter with cheap TCP/CONNECT sweep before expensive HTTP checks (matches the monosans-tier approach).
3. **Parser traps** — `#`-comment headers (Databay, VPSLab, hproxy), scheme prefixes (Proxifly, monosans all.txt), 3-field `ip:port:COUNTRY` (hideip.me), NDJSON (fate0), base64 attributes (advanced.name), JS-obfuscated ports (spys.one), embedded textarea TXT blocks (free-proxy-list.net).
4. **UA/browser requirements** — GeoNode needs a browser UA; kuaidaili/hide.mn/freeproxylists need a real browser; multiproxy.org serves HTML with a 200 — content-type check it.
5. **"Free SOCKS5 with user:pass" red flag** — circulated credential-bearing lists are leaked paid-service creds, not grants; skip them.
6. **GitHub caching** — raw files refresh with the repo; jsDelivr mirrors lag (pin `@commit` for consistency).

## 8. Full dead/blocked registry (do not schedule scrapers against these)

Dead: jetkai/proxy-list (stale 2023, output/ 404) · roosterkid/ACTIVE-PROXY-LIST (repo deleted) · fate0/getproxy (archived 2022) · cyberproxy.me · kproxylist.com · spatine.com · free-proxy-list.io · raw.openproxy.list.xyz / openproxy.space (521) · proxy-list.download (502) · multiproxy.org (parked) · api.getproxylist.com (521) · sockslist.net (parked) · dailyproxylist.com (for sale) · hidden-werx.nl · hide.pro (410) · socks23.com · proxy66.* · basicDNS socks5 list (never findable) · geosurf.com · netnut.io · egress.com · virtuino.net · cyberocean.io · quarkgrid.com · highproxy.net · grabaloop.com · proxyuptime.com · proxying.io · Mrc0113/socks5 (deleted).
Blocked-for-scrapers (browser required): kuaidaili.com · hidemy.name/hide.mn · freeproxylists.net · proxy-listen.de (expired cert + 503 as of 2026-09-28).
Cross-reference — **unverified** (not dead, not blocked): proxydb.net — 2026-09-28 connection timeouts from the sweep host only; DNS resolves, no TCP handshake on 80 or 443, and it served 32 rows/page on 2026-09-12. Row stays in §2; re-probe from a second vantage point before treating as dead.
Country subdomains dead: us.free-proxy-list.net · socks5.free-proxy-list.net · scoped-v3.free-proxy-list.net.

## 9. Cross-lane duplicates (hostname-keyed merge map)

Scout lanes overlap by design; each host appears **once** in this catalog. Seen-in-N-lanes markers:

| Host / repo | Seen in lanes | Catalog home |
|---|---|---|
| api.proxyscrape.com / docs.proxyscrape.com | TxtFeeds, DevTierAPIs, HtmlLists (SPA page) | §1 (SPA note §2, Account API §6) |
| proxylist.geonode.com + app.geonode.com | TxtFeeds, SignupGrants | §1 + §5 (1,500 req/mo tier) |
| databay.com API + databay-labs GitHub repo | TxtFeeds, GitHubFeeds | §1 (API) / §3 (repo — different host, kept separate) |
| proxy.webshare.io API | SignupGrants, DevTierAPIs | §5 row + §6 prose (single provider) |
| raw.githubusercontent: TheSpeedX, monosans, VPSLabCloud, hookzof, roosterkid/openproxylist | TxtFeeds, GitHubFeeds, SocksNiche | §3 once each |
| spys.one / spys.me | HtmlLists, SocksNiche | §2 |
| 89ip.cn, ip3366.net, kuaidaili.com, hide.mn | HtmlLists, SocksNiche | §2 |
| proxy-list.download | TxtFeeds, HtmlLists | §8 dead (single registry row) |
| fate0/proxylist (live) vs fate0/getproxy (archived) | TxtFeeds vs GitHubFeeds | §3 / §8 — distinct repos, not a dup |
| mmpx12/proxy-list (wired) vs ShiftyTR/Proxy-List (rejected, stale) | GitHubFeeds | §3 / §3a — **same upstream, not a dup worth wiring twice.** Verified 2026-09-28: `comm -12` on the two socks4 files returns 661/661 identical lines (mmpx12 has one extra), and socks5 279/279. mmpx12 is the live copy (daily refresh); ShiftyTR is a 2023-08-11 fossil of the same scrape. Only the live one is wired. |

## 10. Independent verification log

### 2026-09-28 — Wave-2 new-source discovery (post-catalog sweep)

16 candidate repos/API endpoints shortlisted from a web sweep of keyless plain-GET proxy sources, then probed with the two-command pattern (HTTP status + distinct `ip:port` token count ≥10) and a `git ls-remote --symref` branch-first pass. Survivors (all bare `ip:port`, no creds, HTTP/HTTPS/SOCKS4/SOCKS5 only): **ErcinDedeoglu/proxies** (`main`; 64,902 / 3,185 / 22,154 / 48,230 tokens), **TuanMinPay/live-proxy** (`master`; 8,556 / 6,223 / 29,636, no https file), **Zaeem20/FREE_PROXIES_LIST** (`master`; 191 / 576 / 86 / 333), **mmpx12/proxy-list** (`master`; 440 / 213 / 661 / 279). Rejections and their evidence are in §3a — notably three large-token feeds (ShiftyTR 823, proxy4parsing 19,031, clarketm 400) that pass the size gate but whose last commit is 2023–2024, so they were classified stale rather than wired. All four survivors were wired as `gh!` rows in `github_feeds.rs` and smoke-run: exit 0 with 1,186 / 44,409 / 1,591 / 138,457 output lines. No new §1 keyless API or §2 HTML-page source cleared the gate this wave — the one API candidate (sockslist.us) 404s on every documented path.

**roosterkid/openproxylist re-scoped in the same wave (sibling-lane discovery).** Branch resolved to `main` via `git ls-remote --symref`. `SOCKS5_RAW.txt` still serves HTTP 200 but only **4** data rows — below the ≥10-row gate, so it is left **unwired** (no scheme override, no `gh!` row), matching the code comment in `github_feeds.rs`. The two sibling lanes that do clear the gate ship: **`SOCKS4_RAW.txt` 149 rows** (socks4) and **`HTTPS_RAW.txt` 59 rows** (https). `HTTP_RAW.txt` and `ALL_PROXIES_RAW.txt` 404 and are unwired. This is why the §3 catalog row lists the two wired files rather than the decayed SOCKS5 lane.

### 2026-09-28 — full §1/§2/§3 re-fetch (this sweep, direct curl, scout labels not trusted)

All 33 §1/§2/§3 entry points re-fetched directly. Counts: ProxyScrape v4 → 1,298 ip:port (second back-to-back fetch 1,527; the endpoint serves a fresh snapshot per request, `cache-control: no-store`); `ssl=yes` → 636 then 475; `ssl=true` → HTTP 400 `Invalid ssl, please use yes or no.` · GeoNode JSON → total=950, fresh lastChecked · Databay API → valid JSON, total=6,048, 500 rows · PubProxy TXT → 2 real ip:port rows (`93.171.157.249:8080`, `113.204.79.230:9091`) · Proxifly jsDelivr → 5,306 · free-proxy-list.net → 303 textarea rows · **free-proxy-list.net us-proxy sub-page → 200, `<textarea>` present, 200 rows** · spys.me/proxy.txt → 400 · **spys.me/socks.txt → 200, 400 unique `ip:port` tokens** · spys.one → 301 → spys.one/en/, 42 KB with **0** ip:port (ports genuinely JS-computed, as documented) · advanced.name → 100 `data-ip` attrs · 89ip.cn → 41 rows · ip3366.net (http://, gb2312) → 16 `<tr>` = 15 data rows, cell-split ip|port, last-verified 2026/9/3–9/4 · Scrappey → 180 `data-protocol` rows (60 http / 60 socks4 / 60 socks5) · kuaidaili → 200 but 983-byte EdgeOne challenge stub · hide.mn → 403 · freeproxylists.net → 403 · proxy-listen.de → expired cert, 503 with `-k` · proxyscrape.com HTML → 274 KB SPA, 0 rows · TheSpeedX → 2,681 · hproxy → 18,997 · monosans → 329 · databay-labs → 3,480 · xyzs996 socks5 → 543 · ALIILAPRO → 990 · VPSLabCloud → 2,003 · iplocate → 574 · fate0/proxylist → 251 NDJSON · vakhov → 722 · sunny9577 → 1,957 · zloi-user/hideip.me → 194 · hookzof → 27,050 · roosterkid/openproxylist SOCKS5_RAW → 4 real ip:port rows (SOCKS4_RAW 149 / HTTPS_RAW 59 — see the wave-2 discovery log above).

That is **33 `·`-separated entries**, counted the same way as the pre-fix line (31 segments, header 31). The lead segment bundles the three ProxyScrape endpoints (`v4`, `ssl=yes`, and the `ssl=true` 400 probe) as one entry, exactly as the pre-fix line did, so this count is in segments, not raw endpoints. The four wave-2 §3 rows this branch adds (ErcinDedeoglu/proxies, TuanMinPay/live-proxy, Zaeem20/FREE_PROXIES_LIST, mmpx12/proxy-list) are verified in the separate discovery log above and are deliberately not part of this list; this count describes only the endpoints fetched by the direct-curl re-fetch itself, not the catalog's total row count.

**Result: every "live" row held except two.** `proxydb.net` could not be verified — connection-level timeouts from the sweep host only (see §2 and the cross-reference in §8). `proxy-listen.de` is now **blocked (cert + 503)**, worse than the cert-only status it previously carried. All other statuses unchanged. Pools fluctuate; counts are point-in-time.

### 2026-09-12 — prior parent re-fetch (kept for history)

15 endpoints spot-checked with direct curl: ProxyScrape v4 → 883 ip:port · GeoNode JSON → total=983, fresh lastChecked · Databay API → total=5,544; socks5.txt → 558 rows · hproxy raw → 4,321 · TheSpeedX → 2,562 · monosans → 426 · hookzof → 223 · free-proxy-list.net → 300 rows · proxydb.net → 32 IP rows/page + protocol cols, 6,394 total marker · spys.me/proxy.txt → 400 · advanced.name → 100 base64 `data-ip` attrs decode clean · Proxifly jsDelivr → 681 · 0xRadikal configs.txt → 11,752 URIs · Webshare API unauthenticated → HTTP 401 exactly as documented. **All catalog "live" rows held.** Pools fluctuate; counts are point-in-time.
