//! advanced.name — freeproxy table with server-side base64 cell attributes.
//!
//! The visible cells look empty; the real address is carried per row in
//! base64 attrs (``data-ip`` / ``data-port``). The two attributes sit on
//! SEPARATE ``<td>`` elements of the same row, so they are paired per
//! ``<tr>``-chunk — pairing two document-order lists positionally would let
//! one stray attribute silently bind every later IP to the wrong port.
//! ``?type=`` selects the protocol each page serves.

use std::sync::{Arc, LazyLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use regex::Regex;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;

static IP_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-ip="([^"]+)""#).unwrap());
static PORT_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-port="([^"]+)""#).unwrap());

fn decode(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let padded = format!("{}{}", trimmed, "=".repeat((4 - trimmed.len() % 4) % 4));
    let bytes = B64.decode(&padded).ok()?;
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn parse_attrs(body: &str, default: Option<Scheme>) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for row in body.split("<tr").skip(1) {
        let (Some(ip), Some(port)) = (
            IP_ATTR.captures(row).and_then(|c| decode(&c[1])),
            PORT_ATTR.captures(row).and_then(|c| decode(&c[1])),
        ) else {
            continue;
        };
        if let Some(sc) = default
            && let Some(p) = make_proxy(sc, &ip, &port, None, None, "advancedname")
        {
            out.push(p);
        }
    }
    out
}

pub struct AdvancedName;

impl Provider for AdvancedName {
    fn id(&self) -> &'static str {
        "advancedname"
    }
    fn site(&self) -> String {
        "https://advanced.name/freeproxy".into()
    }
    fn protocols(&self) -> String {
        "http,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "minutes"
    }
    fn requests(&self) -> Vec<Request> {
        [(Scheme::Http, "http"), (Scheme::Socks5, "socks5")]
            .into_iter()
            .map(|(scheme, proto)| {
                Request::new(
                    format!("https://advanced.name/freeproxy?type={proto}"),
                    proto,
                )
                .with(Some(scheme), ParseKind::Custom(parse_attrs))
            })
            .collect()
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(AdvancedName)
}
