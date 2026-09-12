//! Shared parse components — registered only because multiple providers
//! reuse them. A parser needed by a single source belongs in that provider's
//! module (see providers/proxydb.rs for the pattern).
//!
//! All parsers receive the provider id so records carry provenance;
//! provider-local `Custom` parse fns capture their own id constant.

use serde_json::Value;

use crate::model::{ProxyRecord, Scheme};
use crate::normalize::{make_proxy, make_proxy_from_label, validate_host};

const SKIP_FIRST: &[char] = &['#', '<', '>', '%', '!', '|'];

/// One entry token -> record or None. Understands every line flavor seen
/// across TXT feeds:
/// `host:port` / `host:port:user:pass` / `user:pass@host:port` /
/// `scheme://[user:pass@]host:port` / `host:port:COUNTRY` / bare IPv6.
pub fn parse_entry_token(
    token: &str,
    default: Option<Scheme>,
    source: &'static str,
) -> Option<ProxyRecord> {
    let mut t = token.trim().trim_matches(['"', '`']);
    if t.is_empty() || t.starts_with(SKIP_FIRST) {
        return None;
    }

    let mut scheme = default;
    let mut user: Option<&str> = None;
    let mut pass: Option<&str> = None;

    if let Some((head, rest)) = t.split_once("://") {
        scheme = Some(Scheme::from_label(head)?);
        t = rest.trim_end_matches('/');
    }
    if let Some((creds, rest)) = t.rsplit_once('@') {
        t = rest;
        match creds.split_once(':') {
            Some((u, p)) => {
                user = Some(u);
                pass = Some(p);
            }
            None if !creds.is_empty() => user = Some(creds),
            _ => {}
        }
    }

    let parts: Vec<&str> = t.split(':').collect();
    let (host, port): (&str, &str) = match parts.len() {
        2 => (parts[0], parts[1]),
        3 => {
            let extra = parts[2];
            if !extra.is_empty() && extra.chars().all(|c| c.is_ascii_digit()) {
                return None; // ambiguous host:port:number
            }
            if let Some(sc) = Scheme::from_label(extra) {
                scheme = Some(sc); // host:port:proto
            } // else: country label, ignored
            (parts[0], parts[1])
        }
        4 if user.is_none() => {
            user = Some(parts[2]);
            pass = Some(parts[3]);
            (parts[0], parts[1])
        }
        n if n > 4 => {
            // unbracketed IPv6 + port (IPv6 alone can't parse; the gate drops it)
            let (last, head) = t.rsplit_once(':')?;
            if last.is_empty() || !last.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            (head, last)
        }
        _ => return None,
    };
    make_proxy_from_label(None, scheme, host, port, user, pass, source)
}

fn line_tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split([' ', '\t', '\u{b}', '\u{c}', '\r', ',', ';'])
        .filter(|t| !t.is_empty())
}

pub fn entries(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for line in body.lines() {
        for token in line_tokens(line) {
            if let Some(p) = parse_entry_token(token, default, source) {
                out.push(p);
            }
        }
    }
    out
}

const HOST_KEYS: [&str; 7] = [
    "ip",
    "host",
    "proxy",
    "addr",
    "address",
    "server",
    "ip_address",
];
const USER_KEYS: [&str; 4] = ["username", "user", "login", "proxy_username"];
const PASS_KEYS: [&str; 4] = ["password", "pass", "pwd", "proxy_password"];

fn text_field<'a>(
    obj: &'a serde_json::Map<String, Value>,
    keys: &[&'static str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| obj.get(*k))
        .and_then(|v| v.as_str())
}

/// One JSON record (or embedded entry string) -> zero or more proxies.
/// A record declaring multiple protocols emits one proxy per scheme; the
/// default scheme is only a fallback.
pub fn record_proxies(
    item: &Value,
    default: Option<Scheme>,
    source: &'static str,
) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    match item {
        Value::String(s) => {
            if let Some(p) = parse_entry_token(s, default, source) {
                out.push(p);
            }
            return out;
        }
        Value::Object(obj) => {
            let Some(host) = HOST_KEYS
                .iter()
                .find_map(|k| obj.get(*k).and_then(|v| v.as_str()))
            else {
                return out;
            };
            if validate_host(host).is_none() {
                return out;
            }
            let port = match obj.get("port") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => return out,
            };
            let user = text_field(obj, &USER_KEYS);
            let pass = text_field(obj, &PASS_KEYS);

            let mut declared: Vec<Scheme> = ["protocols", "schemes"]
                .iter()
                .find_map(|k| obj.get(*k).and_then(|v| v.as_array()))
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().and_then(Scheme::from_label))
                        .collect()
                })
                .unwrap_or_default();
            if declared.is_empty() {
                declared = ["protocol", "type", "scheme"]
                    .iter()
                    .filter_map(|k| obj.get(*k).and_then(|v| v.as_str()))
                    .filter_map(Scheme::from_label)
                    .take(1)
                    .collect();
            }
            let schemes: Vec<Scheme> = if declared.is_empty() {
                default.into_iter().collect()
            } else {
                declared.sort();
                declared.dedup();
                declared
            };
            for sc in schemes {
                if let Some(p) = make_proxy(sc, host, &port, user, pass, source) {
                    out.push(p);
                }
            }
        }
        _ => {}
    }
    out
}

fn unwrap_records(data: Value) -> Vec<Value> {
    const ENVELOPE_KEYS: [&str; 8] = [
        "proxies", "data", "result", "results", "list", "items", "nodes", "output",
    ];
    match data {
        Value::Array(items) => items,
        Value::Object(mut map) => {
            for key in ENVELOPE_KEYS {
                if let Some(Value::Array(items)) = map.remove(key) {
                    return items;
                }
            }
            // first list-of-objects anywhere
            map.into_values()
                .find_map(|v| match v {
                    Value::Array(items) if items.first().is_some_and(|i| i.is_object()) => {
                        Some(items)
                    }
                    _ => None,
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

pub fn json(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    let Ok(data) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    unwrap_records(data)
        .iter()
        .flat_map(|item| record_proxies(item, default, source))
        .collect()
}

pub fn ndjson(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    body.lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .flat_map(|item| record_proxies(&item, default, source))
        .collect()
}

/// Dispatch on payload shape: JSON document -> records, else lines. Covers
/// providers whose endpoint flavor changes without warning.
pub fn auto(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    if matches!(body.trim_start().chars().next(), Some('[') | Some('{')) {
        json(body, default, source)
    } else {
        entries(body, default, source)
    }
}
