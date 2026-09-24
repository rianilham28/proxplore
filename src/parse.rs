//! Shared parse components — registered only because multiple providers
//! reuse them. A parser needed by a single source belongs in that provider's
//! module (see providers/proxydb.rs for the pattern).
//!
//! All parsers receive the provider id so records carry provenance;
//! provider-local `Custom` parse fns capture their own id constant.

use serde_json::Value;

use crate::model::{ProxyRecord, Scheme};
use crate::normalize::{make_proxy, make_proxy_from_label, validate_host};

/// Upper bound on records retained from one response. Parsing stops silently
/// here; the drain layer reports truncation to callers.
pub const MAX_PAGE_RECORDS: usize = 100_000;
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
    let has_userinfo = t.contains('@');
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
    if has_userinfo && user.is_none() {
        return None;
    }

    if let Some((bracketed, port)) = t.rsplit_once("]:") {
        let host = bracketed.strip_prefix('[')?;
        return make_proxy_from_label(None, scheme, host, port, user, pass, source);
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
    'outer: for line in body.lines() {
        for token in line_tokens(line) {
            if let Some(p) = parse_entry_token(token, default, source) {
                out.push(p);
                if out.len() == MAX_PAGE_RECORDS {
                    break 'outer;
                }
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
        .find_map(|k| obj.get(*k).and_then(|value| value.as_str()))
}

fn endpoint_without_userinfo(raw: &str) -> std::borrow::Cow<'_, str> {
    if let Some((prefix, endpoint)) = raw.rsplit_once('@') {
        return match prefix.split_once("://") {
            Some((scheme, _)) => std::borrow::Cow::Owned(format!("{scheme}://{endpoint}")),
            None => std::borrow::Cow::Borrowed(endpoint),
        };
    }
    // The legacy tail count is meaningful only after removing the scheme.
    // This prevents the delimiter in :// from mangling endpoint forms, while
    // still allowing scheme://host:port:user:pass to yield to dedicated auth.
    // A three-colon authority may also be compressed IPv6, so validate it as
    // a standalone host before discarding its credential pair.
    let (scheme, authority) = match raw.split_once("://") {
        Some((scheme, authority)) => (Some(scheme), authority),
        None => (None, raw),
    };
    if authority.matches(':').count() == 3 && validate_host(authority).is_none() {
        let mut fields = authority.split(':');
        if let (Some(host), Some(port)) = (fields.next(), fields.next()) {
            return std::borrow::Cow::Owned(match scheme {
                Some(scheme) => format!("{scheme}://{host}:{port}"),
                None => format!("{host}:{port}"),
            });
        }
    }
    std::borrow::Cow::Borrowed(raw)
}

fn declared_schemes(obj: &serde_json::Map<String, Value>, default: Option<Scheme>) -> Vec<Scheme> {
    let mut declared: Vec<Scheme> = ["protocols", "schemes"]
        .iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_array))
        .map(|list| {
            list.iter()
                .filter_map(|value| value.as_str().and_then(Scheme::from_label))
                .collect()
        })
        .unwrap_or_default();
    if declared.is_empty() {
        declared = ["protocol", "type", "scheme"]
            .iter()
            .filter_map(|key| obj.get(*key).and_then(Value::as_str))
            .filter_map(Scheme::from_label)
            .take(1)
            .collect();
    }
    if declared.is_empty() {
        declared = default.into_iter().collect();
    }
    declared.sort();
    declared.dedup();
    declared
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
            if s == s.trim()
                && let Some(p) = parse_entry_token(s, default, source)
            {
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
            if host != host.trim() {
                return out;
            }
            let user = text_field(obj, &USER_KEYS);
            let pass = text_field(obj, &PASS_KEYS);
            let dedicated_auth = user.is_some() || pass.is_some();
            // Dedicated JSON fields are the explicit credential signal. Strip
            // embedded userinfo before endpoint parsing so a malformed loser
            // cannot veto a usable dedicated pair.
            let endpoint = if dedicated_auth {
                endpoint_without_userinfo(host)
            } else {
                std::borrow::Cow::Borrowed(host)
            };
            let schemes = declared_schemes(obj, default);
            let scheme_self_describing = endpoint.contains("://");
            let self_describing = scheme_self_describing || endpoint.contains(']');
            let has_separate_port = matches!(
                obj.get("port"),
                Some(Value::String(_)) | Some(Value::Number(_))
            );
            let (endpoint_accepted, produced) = {
                let mut endpoint_accepted = false;
                let mut produced = false;
                let mut handle_attempt = |attempt: Option<Scheme>| -> bool {
                    let Some(mut parsed) = parse_entry_token(&endpoint, attempt, source) else {
                        return false;
                    };
                    if has_separate_port && !self_describing && validate_host(&endpoint).is_some() {
                        return false;
                    }
                    endpoint_accepted = true;
                    if !self_describing && let Some(scheme) = attempt {
                        parsed.scheme = scheme;
                    }
                    let record = if dedicated_auth {
                        make_proxy(
                            parsed.scheme,
                            &parsed.host,
                            &parsed.port.to_string(),
                            user,
                            pass,
                            source,
                        )
                    } else {
                        Some(parsed)
                    };
                    let Some(record) = record else {
                        return dedicated_auth;
                    };
                    produced = true;
                    out.push(record);
                    scheme_self_describing || out.len() == MAX_PAGE_RECORDS
                };

                if scheme_self_describing || schemes.is_empty() {
                    handle_attempt(None);
                } else {
                    for scheme in &schemes {
                        if handle_attempt(Some(*scheme)) {
                            break;
                        }
                    }
                }
                (endpoint_accepted, produced)
            };
            if (dedicated_auth && endpoint_accepted) || produced {
                return out;
            }
            if validate_host(&endpoint).is_none() {
                return out;
            }
            let port = match obj.get("port") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => return out,
            };
            for scheme in schemes {
                if let Some(record) = make_proxy(scheme, &endpoint, &port, user, pass, source) {
                    out.push(record);
                    if out.len() == MAX_PAGE_RECORDS {
                        break;
                    }
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
            let mut recognized_envelope = false;
            for key in ENVELOPE_KEYS {
                if let Some(value) = map.remove(key) {
                    recognized_envelope = true;
                    if let Value::Array(items) = value {
                        return items;
                    }
                }
            }
            if let Some(items) = map.values().find_map(|value| match value {
                Value::Array(items) if items.first().is_some_and(|item| item.is_object()) => {
                    Some(items.clone())
                }
                _ => None,
            }) {
                return items;
            }
            if recognized_envelope {
                return Vec::new();
            }
            vec![Value::Object(map)]
        }
        _ => Vec::new(),
    }
}

pub fn json(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    let Ok(data) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in unwrap_records(data) {
        for record in record_proxies(&item, default, source) {
            out.push(record);
            if out.len() == MAX_PAGE_RECORDS {
                return out;
            }
        }
    }
    out
}

pub fn ndjson(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    let mut out = Vec::new();
    for line in body
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
    {
        for record in record_proxies(&line, default, source) {
            out.push(record);
            if out.len() == MAX_PAGE_RECORDS {
                return out;
            }
        }
    }
    out
}

/// Dispatch on payload shape: JSON document -> records, else lines. Covers
/// providers whose endpoint flavor changes without warning.
pub fn auto(body: &str, default: Option<Scheme>, source: &'static str) -> Vec<ProxyRecord> {
    if body.trim_start().starts_with('{') {
        if serde_json::from_str::<Value>(body).is_ok() {
            json(body, default, source)
        } else {
            ndjson(body, default, source)
        }
    } else if body.trim_start().starts_with('[') {
        json(body, default, source)
    } else {
        entries(body, default, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(records: Vec<ProxyRecord>) -> Vec<String> {
        records.into_iter().map(|record| record.url()).collect()
    }

    #[test]
    fn entries_extracts_valid_tokens_from_mixed_line_list() {
        let body = "# feed header\n  8.8.8.8:8080  \n\nnot a proxy\nhttps://1.1.1.1:3128\r\n# trailing comment";

        assert_eq!(
            urls(entries(body, Some(Scheme::Http), "fixture")),
            ["http://8.8.8.8:8080", "https://1.1.1.1:3128"]
        );
    }
    #[test]
    fn bracket_json_authorities_follow_declared_schemes() {
        let default = json(
            r#"{"ip":"[2a01:4f8::1]:8080"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let protocols = json(
            r#"{"ip":"[2a01:4f8::1]:8080","protocols":["http","socks5"]}"#,
            None,
            "fixture",
        );
        let credentialed = json(
            r#"{"host":"u:p@[2a01:4f8::1]:8080"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let no_scheme = json(r#"{"ip":"[2a01:4f8::1]:8080"}"#, None, "fixture");
        assert_eq!(urls(default), ["http://[2a01:4f8::1]:8080"]);
        assert_eq!(
            urls(protocols),
            ["http://[2a01:4f8::1]:8080", "socks5://[2a01:4f8::1]:8080"]
        );
        assert_eq!(urls(credentialed), ["http://u:p@[2a01:4f8::1]:8080"]);
        assert!(no_scheme.is_empty());
    }

    #[test]
    fn json_entry_values_reject_surrounding_whitespace() {
        let direct = json(r#"[" u@1.2.3.4:8080"]"#, Some(Scheme::Http), "fixture");
        let clean = json(r#"["u@1.2.3.4:8080"]"#, Some(Scheme::Http), "fixture");
        let host = json(r#"{"host":"1.2.3.4:8080 "}"#, Some(Scheme::Http), "fixture");
        let entries = parse_entry_token("  1.2.3.4:8080  ", Some(Scheme::Http), "fixture");
        assert!(direct.is_empty());
        assert_eq!(urls(clean), ["http://u@1.2.3.4:8080"]);
        assert!(host.is_empty());
        assert!(entries.is_some());
    }

    #[test]
    fn json_reads_array_and_known_envelopes() {
        let array = json(
            r#"[{"ip":"8.8.8.8","port":8080},{"host":"1.1.1.1","port":"3128"}]"#,
            Some(Scheme::Socks4),
            "fixture",
        );
        let envelope = json(
            r#"{"data":[{"ip":"9.9.9.9","port":1080,"protocol":"socks5"}]}"#,
            None,
            "fixture",
        );

        assert_eq!(
            urls(array),
            ["socks4://8.8.8.8:8080", "socks4://1.1.1.1:3128"]
        );
        assert_eq!(urls(envelope), ["socks5://9.9.9.9:1080"]);
    }

    #[test]
    fn json_returns_no_records_for_wrong_root_or_missing_records() {
        let wrong_root = json(r#"{"data":"8.8.8.8:8080"}"#, Some(Scheme::Http), "fixture");
        let missing_records = json(r#"{"total":0}"#, Some(Scheme::Http), "fixture");

        assert!(wrong_root.is_empty());
        assert!(missing_records.is_empty());
    }

    #[test]
    fn ndjson_skips_malformed_lines_and_accepts_trailing_newline() {
        let body = concat!(
            "{\"ip\":\"8.8.8.8\",\"port\":8080,\"protocol\":\"http\"}\n",
            "{not json}\n",
            "{\"host\":\"1.1.1.1\",\"port\":3128,\"type\":\"https\"}\n"
        );

        assert_eq!(
            urls(ndjson(body, None, "fixture")),
            ["http://8.8.8.8:8080", "https://1.1.1.1:3128"]
        );
    }

    #[test]
    fn auto_routes_array_json_and_plain_lines_by_first_content() {
        let json_body = r#"[{"ip":"8.8.8.8","port":8080,"protocol":"http"}]"#;
        let line_body = "1.1.1.1:3128\n9.9.9.9:1080";

        assert_eq!(
            urls(auto(json_body, None, "fixture")),
            ["http://8.8.8.8:8080"]
        );
        assert_eq!(
            urls(auto(line_body, Some(Scheme::Socks5), "fixture")),
            ["socks5://1.1.1.1:3128", "socks5://9.9.9.9:1080"]
        );
    }

    #[test]
    fn auto_distinguishes_json_ndjson_and_pretty_printed_json() {
        let one_record = r#"{"ip":"8.8.8.8","port":8080,"protocol":"http"}"#;
        let ndjson_body = format!("{one_record}\n{one_record}");
        let pretty_json = r#"{
  "data": [
    {"ip": "1.1.1.1", "port": 3128, "protocol": "https"}
  ]
}"#;

        assert_eq!(
            urls(auto(&ndjson_body, None, "fixture")),
            ["http://8.8.8.8:8080", "http://8.8.8.8:8080"]
        );
        assert_eq!(
            urls(auto(pretty_json, None, "fixture")),
            ["https://1.1.1.1:3128"]
        );
    }

    #[test]
    fn parse_entry_token_handles_bracketed_ipv6_and_legacy_fields() {
        assert_eq!(
            urls(vec![
                parse_entry_token("http://[2a01:4f8::1]:8080", None, "fixture").unwrap()
            ]),
            ["http://[2a01:4f8::1]:8080"]
        );
        assert_eq!(
            urls(vec![
                parse_entry_token("u:p@[2a01:4f8::1]:8080", Some(Scheme::Http), "fixture").unwrap()
            ]),
            ["http://u:p@[2a01:4f8::1]:8080"]
        );
        assert_eq!(
            urls(vec![
                parse_entry_token("1.2.3.4:8080:user:pass", Some(Scheme::Http), "fixture").unwrap()
            ]),
            ["http://user:pass@1.2.3.4:8080"]
        );

        assert!(parse_entry_token("http://@[1.2.3.4]:8080", None, "fixture").is_none());
        assert!(parse_entry_token("@1.2.3.4:8080", Some(Scheme::Http), "fixture").is_none());
    }

    #[test]
    fn json_accepts_root_objects_and_embedded_entry_hosts() {
        let root = json(
            r#"{"username":null,"user":"alice","ip":"1.2.3.4","port":8080,"password":"p"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let embedded = json(
            r#"{"proxy":"http://1.2.3.4:8080"}"#,
            Some(Scheme::Http),
            "fixture",
        );

        assert_eq!(urls(root), ["http://alice:p@1.2.3.4:8080"]);
        assert_eq!(urls(embedded), ["http://1.2.3.4:8080"]);

        let bare = json(
            r#"{"ip":"2a01:4f8::1","port":8080}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let bracketed = json(
            r#"{"ip":"[2a01:4f8::1]","port":8080}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(urls(bare), ["http://[2a01:4f8::1]:8080"]);
        assert_eq!(urls(bracketed), ["http://[2a01:4f8::1]:8080"]);

        let no_default = json(r#"{"proxy":"socks5://1.2.3.4:1080"}"#, None, "fixture");
        assert_eq!(urls(no_default), ["socks5://1.2.3.4:1080"]);
    }

    #[test]
    fn embedded_endpoints_preserve_dedicated_credentials() {
        let dedicated = json(
            r#"{"host":"1.2.3.4:8080","username":"u","password":"p"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let overrides = json(
            r#"{"proxy":"embedded:pass@1.2.3.4:8080","username":"u","password":"p"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let invalid = json(
            r#"{"host":"1.2.3.4:8080","username":"u","password":" "}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let protocols = json(
            r#"{"ip":"1.2.3.4:8080","protocols":["http","socks5"]}"#,
            Some(Scheme::Http),
            "fixture",
        );

        assert_eq!(urls(dedicated), ["http://u:p@1.2.3.4:8080"]);
        assert_eq!(urls(overrides), ["http://u:p@1.2.3.4:8080"]);
        assert!(invalid.is_empty());
        assert_eq!(
            urls(protocols),
            ["http://1.2.3.4:8080", "socks5://1.2.3.4:8080"]
        );
        let credentialed = json(
            r#"{"host":"u:p@1.2.3.4:8080"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let credentialed_ipv6 = json(
            r#"{"host":"u:p@[2a01:4f8::1]:8080"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let embedded_port = json(
            r#"{"host":"1.2.3.4:8080","port":9999}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(urls(credentialed), ["http://u:p@1.2.3.4:8080"]);
        assert_eq!(urls(credentialed_ipv6), ["http://u:p@[2a01:4f8::1]:8080"]);
        assert_eq!(urls(embedded_port), ["http://1.2.3.4:8080"]);

        let full_ipv6 = json(
            r#"{"ip":"2a01:4f8:0:0:0:0:0:1","port":8080}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let labeled_protocols = json(
            r#"{"ip":"1.2.3.4:8080:http","protocols":["http","socks5"]}"#,
            None,
            "fixture",
        );
        assert_eq!(urls(full_ipv6), ["http://[2a01:4f8::1]:8080"]);

        let bracketed_port = json(
            r#"{"ip":"[2a01:4f8::1]:8080","port":9999}"#,
            Some(Scheme::Http),
            "fixture",
        );

        let unschemed_protocols = json(
            r#"{"ip":"1.2.3.4:8080","protocols":["http","socks5"]}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let compressed_ipv6 = json(
            r#"{"ip":"2a01:4f8::1","port":8080}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(
            urls(unschemed_protocols),
            ["http://1.2.3.4:8080", "socks5://1.2.3.4:8080"]
        );
        assert_eq!(urls(compressed_ipv6), ["http://[2a01:4f8::1]:8080"]);
        assert_eq!(urls(bracketed_port), ["http://[2a01:4f8::1]:8080"]);
        assert_eq!(
            urls(labeled_protocols),
            ["http://1.2.3.4:8080", "socks5://1.2.3.4:8080"]
        );

        let credentialed_port = json(
            r#"{"host":"u:p@1.2.3.4:8080","port":8080}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let duplicate_protocols = json(
            r#"{"ip":"1.2.3.4:8080","protocols":["http","http"],"port":8080}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(urls(credentialed_port), ["http://u:p@1.2.3.4:8080"]);
        assert_eq!(urls(duplicate_protocols), ["http://1.2.3.4:8080"]);

        let malformed_embedded_wins_dedicated = json(
            r#"{"host":"u:bad%@1.2.3.4:8080","username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let malformed_scheme_embedded = json(
            r#"{"proxy":"http://u:bad%@1.2.3.4:8080","username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let unusable_dedicated = json(
            r#"{"host":"embedded:pass@1.2.3.4:8080","username":"good","password":" "}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let malformed_embedded_no_dedicated = json(
            r#"{"host":"u:bad%@1.2.3.4:8080"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(
            urls(malformed_embedded_wins_dedicated),
            ["http://good:secret@1.2.3.4:8080"]
        );
        assert_eq!(
            urls(malformed_scheme_embedded),
            ["http://good:secret@1.2.3.4:8080"]
        );
        assert!(unusable_dedicated.is_empty());
        assert!(malformed_embedded_no_dedicated.is_empty());

        let malformed_legacy_embedded = json(
            r#"{"host":"1.2.3.4:8080:user:bad%","username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let legacy_embedded_port = json(
            r#"{"host":"1.2.3.4:8080:user:pass","port":9999}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(
            urls(malformed_legacy_embedded),
            ["http://good:secret@1.2.3.4:8080"]
        );
        assert_eq!(
            urls(legacy_embedded_port),
            ["http://user:pass@1.2.3.4:8080"]
        );

        let credentialed_bare_ipv6 = json(
            r#"{"host":"u:p@2a01:4f8::1","port":8080,"username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(
            urls(credentialed_bare_ipv6),
            ["http://good:secret@[2a01:4f8::1]:8080"]
        );

        let three_colon_ipv6 = json(
            r#"{"host":"2a01:4f8::1:2","port":8080,"username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(
            urls(three_colon_ipv6),
            ["http://good:secret@[2a01:4f8::1:2]:8080"]
        );

        let legacy_both_usable = json(
            r#"{"host":"1.2.3.4:8080:user:pass","username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let malformed_scheme_legacy = json(
            r#"{"host":"http://1.2.3.4:8080:user:bad%","username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        let scheme_port_suffix = json(
            r#"{"host":"http://1.2.3.4:8080:http","username":"good","password":"secret"}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert_eq!(
            urls(legacy_both_usable),
            ["http://good:secret@1.2.3.4:8080"]
        );
        assert_eq!(
            urls(malformed_scheme_legacy),
            ["http://good:secret@1.2.3.4:8080"]
        );
        assert_eq!(
            urls(scheme_port_suffix),
            ["http://good:secret@1.2.3.4:8080"]
        );
    }

    #[test]
    fn empty_json_password_is_rejected() {
        let records = json(
            r#"{"host":"u@1.2.3.4:8080","password":""}"#,
            Some(Scheme::Http),
            "fixture",
        );
        assert!(records.is_empty());
    }

    #[test]
    fn empty_entry_password_is_rejected() {
        assert!(parse_entry_token("u:@1.2.3.4:8080", Some(Scheme::Http), "fixture").is_none());
    }

    #[test]
    fn entry_parsing_is_bounded() {
        let body = "1.2.3.4:8080,".repeat(MAX_PAGE_RECORDS + 10);
        assert_eq!(
            entries(&body, Some(Scheme::Http), "fixture").len(),
            MAX_PAGE_RECORDS
        );
    }

    #[test]
    fn json_and_ndjson_parsing_are_bounded() {
        let json_line = r#"{"ip":"1.2.3.4","port":8080}"#;
        let array = format!(
            "[{}]",
            std::iter::repeat_n(json_line, MAX_PAGE_RECORDS + 10)
                .collect::<Vec<_>>()
                .join(",")
        );
        let lines = std::iter::repeat_n(json_line, MAX_PAGE_RECORDS + 10)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            json(&array, Some(Scheme::Http), "fixture").len(),
            MAX_PAGE_RECORDS
        );
        assert_eq!(
            ndjson(&lines, Some(Scheme::Http), "fixture").len(),
            MAX_PAGE_RECORDS
        );
    }
}
