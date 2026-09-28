//! Scrappey — single-page free proxy list tool. Rows are server-rendered
//! <tr data-protocol=...> cells; protocol is a per-row attribute (60/60/60
//! http/socks4/socks5), not a page-level default. Rows carry a stale
//! likelyDead flag upstream; we publish them as-is (proxalyze judges).

use std::sync::Arc;

use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme};
use crate::normalize::make_proxy;

const PAGE: &str = "https://scrappey.com/tools/free-proxy-lists/socks5";

fn parse_rows(body: &str, _default: Option<Scheme>) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for row in body.split("<tr ").skip(1) {
        // rows without data-protocol (the <th> header row) fall through
        let Some(protocol) = attr(row, "data-protocol") else {
            continue;
        };
        let Some(scheme) = Scheme::from_label(protocol) else {
            continue;
        };
        // first two cells are ip and port; later cells carry country,
        // anonymity, last-checked notes.
        let cells = cells(row);
        let (Some(ip), Some(port)) = (cells.first(), cells.get(1)) else {
            continue;
        };
        if let Some(record) = make_proxy(scheme, ip, port, None, None, "scrappey") {
            out.push(record);
        }
    }
    out
}

fn attr<'a>(row: &'a str, name: &str) -> Option<&'a str> {
    let at = row.find(name)?;
    let rest = &row[at + name.len()..];
    let rest = rest.strip_prefix("=\"")?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn cells(row: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for cell in row.split("<td").skip(1) {
        let Some(gt) = cell.find('>') else {
            continue;
        };
        let Some(close) = cell[gt + 1..].find("</td>") else {
            continue;
        };
        out.push(cell[gt + 1..gt + 1 + close].trim());
    }
    out
}

pub struct Scrappey;

impl Provider for Scrappey {
    fn id(&self) -> &'static str {
        "scrappey"
    }
    fn site(&self) -> String {
        PAGE.into()
    }
    fn protocols(&self) -> String {
        "http,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "~15 min"
    }
    fn requests(&self) -> Vec<Request> {
        // None: every row self-describes its protocol via data-protocol
        vec![Request::new(PAGE, "list").with(None, ParseKind::Custom(parse_rows))]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Scrappey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rows_reads_per_row_protocol_and_skips_header() {
        let body = concat!(
            "<table><thead><tr><th>IP</th><th>Port</th></tr></thead><tbody>",
            // real rows carry six cells; only ip/port matter
            "<tr data-protocol=\"http\" data-cc=\"\" data-anonymity=\"\" data-search=\"8.8.8.8:8080\" class=\"x\">",
            "<td class=\"py-2.5 px-4 font-mono\">8.8.8.8</td><td class=\"py-2.5 px-4 font-mono\">8080</td>",
            "<td>—</td><td><span>HTTP</span></td><td>—</td><td>just now</td></tr>",
            "<tr data-protocol=\"socks5\" data-search=\"1.1.1.1:3128\">",
            "<td>1.1.1.1</td><td>3128</td></tr>",
            "<tr data-protocol=\"socks4\" data-search=\"9.9.9.9:1080\">",
            "<td>9.9.9.9</td><td>1080</td></tr>",
            "</tbody></table>",
        );
        assert_eq!(
            parse_rows(body, None)
                .into_iter()
                .map(|r| r.url())
                .collect::<Vec<_>>(),
            [
                "http://8.8.8.8:8080",
                "socks5://1.1.1.1:3128",
                "socks4://9.9.9.9:1080"
            ]
        );
    }
}
