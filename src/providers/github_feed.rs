//! Reusable base for the cron-refreshed GitHub raw ip:port feeds — a large
//! slice of proxplore's providers is "one protocol-pure file per repo path".
//! They differ only by repo/branch/file-set, so they compose this struct
//! (see github_feeds.rs for the table). Providers with a machine-readable
//! sibling set `json_extra` to append a self-describing proxies.json request.

use crate::model::{ParseKind, Provider, Request, Scheme};

pub const RAW_BASE: &str = "https://raw.githubusercontent.com/";

/// One file in the repo. `scheme` is None when every record self-describes
/// its protocol (NDJSON feeds); otherwise it is the default the lane asserts.
pub struct FeedFile {
    pub scheme: Option<Scheme>,
    pub path: &'static str,
}

impl FeedFile {
    pub const fn scheme_and(scheme: Scheme, path: &'static str) -> FeedFile {
        FeedFile {
            scheme: Some(scheme),
            path,
        }
    }
    pub const fn none(path: &'static str) -> FeedFile {
        FeedFile { scheme: None, path }
    }
}

pub struct GithubFeed {
    pub id: &'static str,
    pub repo: &'static str, // "owner/name"
    pub branch: &'static str,
    /// owned so the table macro can build lists at runtime without const-promotion
    pub files: Vec<FeedFile>,
    pub refresh: &'static str,
    pub parser: ParseKind,
    /// Repo also publishes proxies.json (schema-rich, self-describing) —
    /// appended as an extra lane so credentials/geo records survive.
    pub json_extra: bool,
}

pub fn site_of(feed: &GithubFeed) -> String {
    format!("https://github.com/{}", feed.repo)
}

pub fn protocols_of(feed: &GithubFeed) -> String {
    let mut v: Vec<&str> = feed
        .files
        .iter()
        .filter_map(|f| f.scheme.map(Scheme::as_str))
        .collect();
    v.sort_unstable();
    v.dedup();
    if v.is_empty() {
        "http,https,socks4,socks5".into()
    } else {
        v.join(",")
    }
}

pub fn build_requests(feed: &GithubFeed) -> Vec<Request> {
    let mut out: Vec<Request> = feed
        .files
        .iter()
        .map(|f| {
            let label = f
                .scheme
                .map(|s| s.as_str().to_string())
                .unwrap_or_else(|| f.path.to_string());
            Request::new(
                format!("{}{}/{}/{}", RAW_BASE, feed.repo, feed.branch, f.path),
                label,
            )
            .with(f.scheme, feed.parser)
        })
        .collect();
    if feed.json_extra {
        out.push(
            Request::new(
                format!("{}{}/{}/proxies.json", RAW_BASE, feed.repo, feed.branch),
                "json",
            )
            .with(None, ParseKind::Json),
        );
    }
    out
}

impl Provider for GithubFeed {
    fn id(&self) -> &'static str {
        self.id
    }
    fn site(&self) -> String {
        site_of(self)
    }
    fn protocols(&self) -> String {
        protocols_of(self)
    }
    fn refresh(&self) -> &'static str {
        self.refresh
    }
    fn requests(&self) -> Vec<Request> {
        build_requests(self)
    }
}
