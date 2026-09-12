//! ip3366.net — Chinese listing (plain http; TLS chain broken upstream).
//!
//! Rows carry a per-row type column (cells: IP | PORT | anonymity |
//! HTTP/HTTPS | location...), so scheme is read per row and this lane yields
//! both http and https. Exactly 15 rows × 7 pages × 2 pools (stype 1/2);
//! past-end pages CLAMP to page 7 and return 200, so fixed pages are the
//! verified stop — no advance(). Charset is gb2512 but every needed cell is
//! ASCII. Low-volume opportunistic lane (~200 rows/day, checks can run days
//! stale) kept for coverage; rows repeat with 89ip (same operator family).

use std::sync::Arc;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;

const BASE: &str = "http://www.ip3366.net/free/";

fn parse_rows(body: &str, _default: Option<Scheme>) -> Vec<ProxyRecord> {
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
        if cells.len() < 4 {
            continue;
        }
        let scheme = match cells[3].to_ascii_uppercase().as_str() {
            "HTTP" => Scheme::Http,
            "HTTPS" => Scheme::Https,
            _ => continue,
        };
        if let Some(p) = make_proxy(scheme, cells[0], cells[1], None, None, "ip3366") {
            out.push(p);
        }
    }
    out
}

pub struct Ip3366;

impl Provider for Ip3366 {
    fn id(&self) -> &'static str {
        "ip3366"
    }
    fn site(&self) -> String {
        "http://www.ip3366.net/free/".into()
    }
    fn protocols(&self) -> String {
        "http,https".into()
    }
    fn refresh(&self) -> &'static str {
        "daily"
    }
    fn requests(&self) -> Vec<Request> {
        let mut out = Vec::new();
        for stype in 1..=2 {
            for page in 1..=7 {
                out.push(
                    Request::new(
                        format!("{BASE}?stype={stype}&page={page}"),
                        format!("stype={stype} p{page}"),
                    )
                    .with(None, ParseKind::Custom(parse_rows)),
                );
            }
        }
        out
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Ip3366)
}
