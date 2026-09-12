//! Free-Proxy-List.net — server-rendered table pages with a raw export block.
//!
//! The anonymous and SSL pages each embed a "Raw Proxy List" <textarea> of
//! plain ip:port lines (HTML-escaped — unescaped before the shared token
//! parser; cleaner than the table). The socks page is the exception: its
//! textarea is an untagged socks4/socks5 mix, but its TABLE carries a
//! per-row "Proxy Type" column, so that page is parsed from cells instead
//! and self-corrects when Socks5 rows (re)appear. Old country subdomains
//! (us./socks5./scoped-v3.) are NXDOMAIN and unreferenced.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;
use crate::parse::parse_entry_token;

fn unescape(text: &str) -> String {
    text.replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_textarea(body: &str, default: Option<Scheme>) -> Vec<ProxyRecord> {
    let Some(start) = body.find("<textarea") else {
        return Vec::new();
    };
    let rest = &body[start..];
    let Some(open) = rest.find('>') else {
        return Vec::new();
    };
    let Some(close) = rest[open + 1..].find("</textarea>") else {
        return Vec::new();
    };
    let inner = unescape(&rest[open + 1..open + 1 + close]);
    let mut out = Vec::new();
    for line in inner.lines() {
        for token in line.split_whitespace() {
            if let Some(p) = parse_entry_token(token, default, "free-proxy-list-net") {
                out.push(p);
            }
        }
    }
    out
}

fn parse_socks_columns(body: &str, _default: Option<Scheme>) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for row in body.split("<tr").skip(1) {
        let cells: Vec<&str> = row
            .split("<td")
            .skip(1)
            .filter_map(|c| {
                let gt = c.find('>')?;
                let close = c[gt + 1..].find("</td>")?;
                Some(c[gt + 1..gt + 1 + close].trim())
            })
            .collect();
        if cells.len() < 5 {
            continue;
        }
        // Proxy Type cell: "Socks4" / "Socks5"
        let ty = cells[4].to_ascii_lowercase().replace(' ', "");
        let scheme = match ty.as_str() {
            "socks4" => Scheme::Socks4,
            "socks5" => Scheme::Socks5,
            _ => continue,
        };
        if let Some(p) = make_proxy(
            scheme,
            cells[0],
            cells[1],
            None,
            None,
            "free-proxy-list-net",
        ) {
            out.push(p);
        }
    }
    out
}

pub struct FreeProxyListNet;

impl Provider for FreeProxyListNet {
    fn id(&self) -> &'static str {
        "free-proxy-list-net"
    }
    fn site(&self) -> String {
        "https://free-proxy-list.net".into()
    }
    fn protocols(&self) -> String {
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "10 min"
    }
    fn requests(&self) -> Vec<Request> {
        vec![
            Request::new("https://free-proxy-list.net/", "anonymous")
                .with(Some(Scheme::Http), ParseKind::Custom(parse_textarea)),
            Request::new("https://free-proxy-list.net/ssl-proxy.html", "ssl")
                .with(Some(Scheme::Https), ParseKind::Custom(parse_textarea)),
            Request::new("https://free-proxy-list.net/socks-proxy.html", "socks")
                .with(None, ParseKind::Custom(parse_socks_columns)),
        ]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(FreeProxyListNet)
}
