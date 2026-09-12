//! 89ip.cn — Chinese server-rendered proxy listing (layui table).
//!
//! A plain GET serves the table (no JS wall), but the markup needs bespoke
//! handling kept in this module:
//!
//! * Only the FIRST ``<table class="layui-table">`` holds rows — a second
//!   layui-styled table follows with ad content, so the body is sliced at the
//!   next ``</table>``/``<table`` boundary before pairing.
//! * IP and PORT are consecutive ``<td>`` cells padded with newlines/tabs,
//!   matched by a whitespace-tolerant pairing regex rather than cell splits.
//! * No per-row protocol column (the Http/Https split is only an aggregate
//!   count), so every row is tagged with the default scheme "http".
//!
//! Listing is depth-bounded (~4.4k rows / ~110 pages): every page is seeded
//! in parallel — past-end pages answer 200 with an empty table and yield
//! nothing, so termination never depends on HTTP status.

use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;

const SITE: &str = "http://www.89ip.cn/";
const PAGES: usize = 130; // root + index_2..130: ~110 measured, growth slack

static ROWS_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<table[^>]*class="layui-table"[^>]*>"#).unwrap());
static PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<td>\s*(\d{1,3}(?:\.\d{1,3}){3})\s*</td>\s*<td>\s*(\d{2,5})\s*</td>").unwrap()
});

fn rows_table(body: &str) -> &str {
    let Some(m) = ROWS_OPEN.find(body) else {
        return "";
    };
    let rest = &body[m.end()..];
    let end = [rest.find("</table>"), rest.find("<table")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

fn parse_rows(body: &str, _default: Option<Scheme>) -> Vec<ProxyRecord> {
    PAIR.captures_iter(rows_table(body))
        .filter_map(|c| make_proxy(Scheme::Http, &c[1], &c[2], None, None, "89ip"))
        .collect()
}

pub struct E89ip;

impl Provider for E89ip {
    fn id(&self) -> &'static str {
        "89ip"
    }
    fn site(&self) -> String {
        SITE.into()
    }
    fn protocols(&self) -> String {
        "http".into()
    }
    fn refresh(&self) -> &'static str {
        "daily (multiple posts/day)"
    }
    fn max_requests(&self) -> usize {
        PAGES + 10
    }
    fn time_budget(&self) -> std::time::Duration {
        std::time::Duration::from_secs(240)
    }
    fn requests(&self) -> Vec<Request> {
        let urls = std::iter::once(SITE.to_string())
            .chain((2..=PAGES).map(|n| format!("{SITE}index_{n}.html")));
        urls.enumerate()
            .map(|(i, url)| {
                Request::new(url, format!("page={}", i + 1))
                    .with(Some(Scheme::Http), ParseKind::Custom(parse_rows))
            })
            .collect()
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(E89ip)
}
