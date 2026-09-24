//! ProxyDB.net — server-rendered HTML table, continuous re-check.
//!
//! Bespoke parsing lives here (contract policy: don't generalize single-source
//! quirks):
//!
//! * Port cells hide a decoy digit (``<div style="display:none">12</div>
//!   <a>80</a>`` renders as "1280") — the address is therefore read from each
//!   row's ``href="/ip/port#proto"`` link, never from cell text.
//! * The row's protocol comes from that link fragment, falling back to a
//!   word scan of the row, then to the feed default.
//!
//! The browsable listing is depth-bounded (~6.4k rows / 30 per page), so
//! every page is seeded as a parallel request — one sweep of the offset
//! range beats a 220-hop sequential chain, and past-end pages answer with an
//! empty table, yielding nothing. (Reference provider for the bespoke-parse
//! + full-fan-out pattern.)

use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;

const SITE: &str = "https://proxydb.net/";
const PAGE_STEP: usize = 30;
const PAGES: usize = 230; // ~6.4k rows + growth slack

static HREF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href="/([^"]*?)/(\d+)(?:#([^"]*))?""#).unwrap());
static PROTO_WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(socks5|socks4|https|http)\b").unwrap());

fn row_scheme(fragment: Option<&str>, row_html: &str, default: Option<Scheme>) -> Option<Scheme> {
    if let Some(sc) = fragment.and_then(Scheme::from_label) {
        return Some(sc);
    }
    let scan = PROTO_WORD_RE
        .captures(&row_html.to_ascii_lowercase())
        .and_then(|c| Scheme::from_label(&c[1]));
    scan.or(default)
}

fn parse_rows(body: &str, default: Option<Scheme>) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for row in body.split("<tr").skip(1) {
        let Some(c) = HREF_RE.captures(row) else {
            continue;
        };
        let Some(scheme) = row_scheme(c.get(3).map(|m| m.as_str()), row, default) else {
            continue;
        };
        if let Some(p) = make_proxy(scheme, &c[1], &c[2], None, None, "proxydb") {
            out.push(p);
        }
    }
    out
}

pub struct Proxydb;

impl Provider for Proxydb {
    fn id(&self) -> &'static str {
        "proxydb"
    }
    fn site(&self) -> String {
        SITE.into()
    }
    fn protocols(&self) -> String {
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "continuous"
    }
    fn requests(&self) -> Vec<Request> {
        (0..PAGES)
            .map(|i| {
                Request::new(
                    format!("{SITE}?offset={}", i * PAGE_STEP),
                    format!("page={}", i + 1),
                )
                .with(Some(Scheme::Http), ParseKind::Custom(parse_rows))
            })
            .collect()
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Proxydb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rows_uses_href_port_instead_of_decoy_cell_text() {
        let body = concat!(
            "<table><tr><th>IP</th><th>Port</th></tr>",
            "<tr><td>8.8.8.8</td><td><div style=\"display:none\">12</div><a>80</a></td>",
            "<td><a href=\"/8.8.8.8/8080#https\">details</a></td></tr>",
            "<tr><td>1.1.1.1</td><td><div style=\"display:none\">99</div><a>90</a></td>",
            "<td><a href=\"/1.1.1.1/3128#http\">details</a></td></tr></table>"
        );

        assert_eq!(
            parse_rows(body, Some(Scheme::Http))
                .into_iter()
                .map(|record| record.url())
                .collect::<Vec<_>>(),
            ["https://8.8.8.8:8080", "http://1.1.1.1:3128"]
        );
    }
}
