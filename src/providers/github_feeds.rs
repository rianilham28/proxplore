//! The GitHub raw-feed lane — every cron-refreshed repo that publishes one
//! protocol-pure file per scheme. Identity, URLs, branches, and lanes here
//! are verified against the live repo trees (expansion/research sweeps) and
//! mirror the proven Python providers 1:1 (ids keep their exact hyphenation).

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Scheme};
use crate::providers::github_feed::{FeedFile, GithubFeed};

macro_rules! gh {
    ($fname:ident, $id:literal, $repo:literal, $branch:literal, $refresh:literal,
     $parser:expr, $json:literal, [ $( $file:expr ),+ $(,)? ]) => {
        pub fn $fname() -> Arc<dyn Provider> {
            Arc::new(GithubFeed {
                id: $id,
                repo: $repo,
                branch: $branch,
                files: vec![ $( $file ),+ ],
                refresh: $refresh,
                parser: $parser,
                json_extra: $json,
            })
        }
    };
}

const E: ParseKind = ParseKind::Entries; // bare ip:port files

gh!(
    thespeedx,
    "thespeedx",
    "TheSpeedX/PROXY-List",
    "master",
    "daily",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
gh!(
    hproxy,
    "hproxy",
    "hproxy-com/free-proxy-list",
    "main",
    "several×/day",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Https, "https.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
// monosans TXT lines may carry user:pass@; proxies.json adds geo/cred fields.
gh!(
    monosans,
    "monosans",
    "monosans/proxy-list",
    "main",
    "hourly re-check",
    E,
    true,
    [
        FeedFile::scheme_and(Scheme::Http, "proxies/http.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "proxies/socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "proxies/socks5.txt"),
    ]
);
// ⚠ default branch is master for databay-labs (verified via repo tree).
gh!(
    databay_labs,
    "databay-labs",
    "databay-labs/free-proxy-list",
    "master",
    "5 min",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
gh!(
    xyzs996,
    "xyzs996",
    "xyzs996/free-proxy-health-list",
    "main",
    "30 min",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Https, "https.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
gh!(
    aliilapro,
    "aliilapro",
    "ALIILAPRO/Proxy",
    "main",
    "hourly",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
// VPSLabCloud splits by anonymity×SSL; the elite/all lanes are the pure ones.
gh!(
    vpslabcloud,
    "vpslabcloud",
    "VPSLabCloud/VPSLab-Free-Proxy-List",
    "main",
    "several×/day",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http_elite.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4_all.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5_all.txt"),
    ]
);
gh!(
    iplocate,
    "iplocate",
    "iplocate/free-proxy-list",
    "main",
    "30 min",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "protocols/http.txt"),
        FeedFile::scheme_and(Scheme::Https, "protocols/https.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "protocols/socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "protocols/socks5.txt"),
    ]
);
// sunny9577 publishes http/socks4/socks5 (no https file — that 404s).
gh!(
    sunny9577,
    "sunny9577",
    "sunny9577/proxy-scraper",
    "master",
    "scheduled",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "generated/http_proxies.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "generated/socks4_proxies.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "generated/socks5_proxies.txt"),
    ]
);
// hideip.me lines are ip:port:COUNTRY_FULL (trailing country dropped by parser).
gh!(
    hideip_me,
    "hideip-me",
    "zloi-user/hideip.me",
    "main",
    "frequent",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
gh!(
    hookzof,
    "hookzof",
    "hookzof/socks5_list",
    "master",
    "CI-refreshed",
    E,
    false,
    [FeedFile::scheme_and(Scheme::Socks5, "proxy.txt"),]
);
// vakhov: the mixed proxylist.txt/json are untaggable — pure per-protocol files only.
gh!(
    vakhov,
    "vakhov",
    "vakhov/fresh-proxy-list",
    "master",
    "hours",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Https, "https.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
// rix4uni: single huge mixed file; its json flags show ~98.6% http, so assert http.
gh!(
    rix4uni,
    "rix4uni",
    "rix4uni/fresh-proxy-list",
    "main",
    "daily",
    E,
    false,
    [FeedFile::scheme_and(Scheme::Http, "proxylist.txt"),]
);
// blitzproxy raw-files/ protocol-pure dumps (out-files/ are near-duplicates).
gh!(
    blitzproxy,
    "blitzproxy",
    "i-am-unbekannt/BLITZPROXY",
    "main",
    "2 days",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "raw-files/raw-http.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "raw-files/raw-socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "raw-files/raw-socks5.txt"),
    ]
);
gh!(
    proxio_io,
    "proxio-io",
    "proxio-io/proxy-list",
    "main",
    "20 min",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Http, "http.txt"),
        FeedFile::scheme_and(Scheme::Https, "https.txt"),
        FeedFile::scheme_and(Scheme::Socks4, "socks4.txt"),
        FeedFile::scheme_and(Scheme::Socks5, "socks5.txt"),
    ]
);
// fate0: one NDJSON stream; every record declares its own type -> scheme None.
gh!(
    fate0,
    "fate0",
    "fate0/proxylist",
    "master",
    "continuous",
    ParseKind::Ndjson,
    false,
    [FeedFile::none("proxy.list"),]
);
// roosterkid/openproxylist: branch `main` verified via `ls-remote --symref` 2026-09-28.
// SOCKS5_RAW decayed to 4 rows (below the >=10 gate) — left unwired; the other two
// lanes cleared it (SOCKS4_RAW 149, HTTPS_RAW 59). HTTP_RAW/ALL_PROXIES_RAW 404.
gh!(
    roosterkid,
    "roosterkid",
    "roosterkid/openproxylist",
    "main",
    "occasional",
    E,
    false,
    [
        FeedFile::scheme_and(Scheme::Socks4, "SOCKS4_RAW.txt"),
        FeedFile::scheme_and(Scheme::Https, "HTTPS_RAW.txt"),
    ]
);

pub fn all() -> Vec<Arc<dyn Provider>> {
    vec![
        thespeedx(),
        hproxy(),
        monosans(),
        databay_labs(),
        xyzs996(),
        aliilapro(),
        vpslabcloud(),
        iplocate(),
        sunny9577(),
        hideip_me(),
        hookzof(),
        vakhov(),
        rix4uni(),
        blitzproxy(),
        proxio_io(),
        fate0(),
        roosterkid(),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn roosterkid_is_registered() {
        let ids: Vec<&str> = crate::providers::all().iter().map(|p| p.id()).collect();
        assert!(ids.contains(&"roosterkid"));
    }

    // Guards the re-scope decision itself: the two lanes that cleared the >=10
    // row gate are fetched, on the probed `main` branch, and the SOCKS5 lane
    // (decayed to 4 rows) stays unwired so no request can ever 404-then-empty it.
    #[test]
    fn roosterkid_fetches_only_the_two_non_decayed_lanes() {
        let p = crate::providers::all()
            .into_iter()
            .find(|p| p.id() == "roosterkid")
            .expect("roosterkid registered");
        let reqs = p.requests();
        let urls: Vec<&str> = reqs.iter().map(|r| r.url.as_str()).collect();
        assert!(urls.contains(
            &"https://raw.githubusercontent.com/roosterkid/openproxylist/main/SOCKS4_RAW.txt"
        ));
        assert!(urls.contains(
            &"https://raw.githubusercontent.com/roosterkid/openproxylist/main/HTTPS_RAW.txt"
        ));
        assert!(
            !urls.iter().any(|u| u.contains("SOCKS5_RAW")),
            "decayed SOCKS5 lane must stay unwired"
        );
        assert!(
            urls.iter().all(|u| u.contains("/main/")),
            "branch must be the probed main"
        );
    }
}
