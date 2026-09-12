//! Normalization — the single validation gate for proxies.
//!
//! Every parser funnels candidates through [`make_proxy`]; anything that is
//! not a usable, sane `scheme://host:port` dies here. Parsers therefore only
//! know how to extract fields, never how to validate them.
//!
//! The IP filter is a BIT-EXACT port of CPython 3.14's `ipaddress`
//! (`is_private | is_reserved | is_loopback | is_link_local | is_multicast |
//! is_unspecified`), generated from the reference implementation itself (see
//! the `tests` module for the differential vectors). Notable consequences:
//! carrier-grade NAT `100.64/10` is KEPT (CPython lists it as "public"),
//! `192.0.0.9-10` are kept (private-range exceptions), IPv4-mapped IPv6
//! delegates to the embedded v4 test, and the v6 reserved table swallows
//! most non-`2000::/3` space.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::model::{ProxyRecord, Scheme};

type Cidr4 = (u32, u8);
type Cidr6 = (u128, u8);

/// CPython 3.14 IPv4Address._constants._private_networks
const V4_PRIVATE: [Cidr4; 14] = [
    (0x00000000, 8),
    (0x0A000000, 8),
    (0x7F000000, 8),
    (0xA9FE0000, 16),
    (0xAC100000, 12),
    (0xC0000000, 24),
    (0xC00000AA, 31),
    (0xC0000200, 24),
    (0xC0A80000, 16),
    (0xC6120000, 15),
    (0xC6336400, 24),
    (0xCB007100, 24),
    (0xF0000000, 4),
    (0xFFFFFFFF, 32),
];
/// ..._private_networks_exceptions (kept despite sitting inside a private range)
const V4_EXCEPTIONS: [Cidr4; 2] = [(0xC0000009, 32), (0xC000000A, 32)];

/// CPython 3.14 IPv6Address._constants._private_networks
const V6_PRIVATE: [Cidr6; 11] = [
    (0x00000000000000000000000000000001, 128),
    (0x00000000000000000000000000000000, 128),
    (0x00000000000000000000FFFF00000000, 96),
    (0x0064FF9B000100000000000000000000, 48),
    (0x01000000000000000000000000000000, 64),
    (0x20010000000000000000000000000000, 23),
    (0x20010DB8000000000000000000000000, 32),
    (0x20020000000000000000000000000000, 16),
    (0x3FFF0000000000000000000000000000, 20),
    (0xFC000000000000000000000000000000, 7),
    (0xFE800000000000000000000000000000, 10),
];
const V6_EXCEPTIONS: [Cidr6; 6] = [
    (0x20010001000000000000000000000001, 128),
    (0x20010001000000000000000000000002, 128),
    (0x20010003000000000000000000000000, 32),
    (0x20010004011200000000000000000000, 48),
    (0x20010020000000000000000000000000, 28),
    (0x20010030000000000000000000000000, 28),
];
/// CPython 3.14 IPv6Address._constants._reserved_networks
const V6_RESERVED: [Cidr6; 15] = [
    (0x00000000000000000000000000000000, 8),
    (0x01000000000000000000000000000000, 8),
    (0x02000000000000000000000000000000, 7),
    (0x04000000000000000000000000000000, 6),
    (0x08000000000000000000000000000000, 5),
    (0x10000000000000000000000000000000, 4),
    (0x40000000000000000000000000000000, 3),
    (0x60000000000000000000000000000000, 3),
    (0x80000000000000000000000000000000, 3),
    (0xA0000000000000000000000000000000, 3),
    (0xC0000000000000000000000000000000, 3),
    (0xE0000000000000000000000000000000, 4),
    (0xF0000000000000000000000000000000, 5),
    (0xF8000000000000000000000000000000, 6),
    (0xFE000000000000000000000000000000, 9),
];

const fn in4(ip: u32, table: &[Cidr4]) -> bool {
    let mut i = 0;
    while i < table.len() {
        let (net, bits) = table[i];
        let mask = if bits == 0 {
            0
        } else {
            u32::MAX << (32 - bits)
        };
        if ip & mask == net & mask {
            return true;
        }
        i += 1;
    }
    false
}

const fn in6(ip: u128, table: &[Cidr6]) -> bool {
    let mut i = 0;
    while i < table.len() {
        let (net, bits) = table[i];
        let mask = if bits == 0 {
            0
        } else {
            u128::MAX << (128 - bits)
        };
        if ip & mask == net & mask {
            return true;
        }
        i += 1;
    }
    false
}

fn v4_unusable(v4: Ipv4Addr) -> bool {
    let ip = u32::from(v4);
    // is_private minus exceptions, plus is_multicast (224/4 sits outside
    // CPython's private list and is a separate predicate)
    (in4(ip, &V4_PRIVATE) && !in4(ip, &V4_EXCEPTIONS)) || (ip >> 28) == 0xE // 224.0.0.0/4
}

fn v6_unusable(v6: Ipv6Addr) -> bool {
    // CPython is_private/is_reserved delegate for IPv4-mapped addresses
    if let Some(inner) = v6.to_ipv4_mapped() {
        return v4_unusable(inner);
    }
    let ip = u128::from(v6);
    in6(ip, &V6_PRIVATE) && !in6(ip, &V6_EXCEPTIONS) || in6(ip, &V6_RESERVED) || (ip >> 120) == 0xFF // ff00::/8 multicast — separate CPython predicate
}

fn ip_unusable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_unusable(v4),
        IpAddr::V6(v6) => v6_unusable(v6),
    }
}

/// Canonical host token: IPv4 dotted, bracketed IPv6 (`[2a01::1]`), or
/// dotted lowercase hostname. Literal IPs are filtered per module docs.
pub fn validate_host(raw: &str) -> Option<String> {
    let h = raw.trim().trim_end_matches('.');
    validate_host_inner(h)
}

fn validate_host_inner(h: &str) -> Option<String> {
    let h = h.trim();
    if h.is_empty() {
        return None;
    }
    if let Some(inner) = h.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let ip: IpAddr = inner.parse().ok()?;
        return if ip_unusable(ip) {
            None
        } else {
            Some(format!("[{ip}]"))
        };
    }
    if h.contains(':') {
        // unbracketed multi-colon -> bare IPv6; std Display already compresses
        let ip: IpAddr = h.parse().ok()?;
        return match ip {
            IpAddr::V6(v6) if !ip_unusable(ip) => Some(format!("[{v6}]")),
            _ => None,
        };
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return if ip_unusable(ip) {
            None
        } else {
            Some(ip.to_string())
        };
    }
    let h = h.to_ascii_lowercase();
    if valid_hostname(&h) { Some(h) } else { None }
}

fn valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

fn clean_cred(raw: Option<&str>) -> Option<String> {
    let v = raw?.trim();
    if v.is_empty()
        || v.chars()
            .any(|c| c.is_whitespace() || c == '@' || c == '/' || c == '\\')
    {
        return None;
    }
    Some(v.to_string())
}

/// Validate + canonicalize one candidate; None = reject. `port` arrives as
/// text straight from the feed.
pub fn make_proxy(
    scheme: Scheme,
    host: &str,
    port: &str,
    user: Option<&str>,
    pass: Option<&str>,
    source: &'static str,
) -> Option<ProxyRecord> {
    let host = validate_host(host)?;
    let port: u16 = port.trim().parse().ok()?;
    if port == 0 {
        return None;
    }
    let user = clean_cred(user);
    Some(ProxyRecord {
        scheme,
        host,
        port,
        pass: if user.is_some() {
            Some(clean_cred(pass).unwrap_or_default())
        } else {
            None
        },
        user,
        source,
    })
}

pub fn make_proxy_from_label(
    scheme_label: Option<&str>,
    fallback: Option<Scheme>,
    host: &str,
    port: &str,
    user: Option<&str>,
    pass: Option<&str>,
    source: &'static str,
) -> Option<ProxyRecord> {
    let scheme = scheme_label.and_then(Scheme::from_label).or(fallback)?;
    make_proxy(scheme, host, port, user, pass, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground truth captured from CPython 3.14's ipaddress module on
    /// 2026-09-12 (`not (is_private | is_reserved | is_loopback |
    /// is_link_local | is_multicast | is_unspecified)`) — this test IS the
    /// Python-parity contract for the gate.
    const VECTORS: &[(&str, bool)] = &[
        // "keep the good ones"
        ("8.8.8.8", true),
        ("1.1.1.1", true),
        ("9.9.9.9", true),
        ("100.63.0.1", true), // just below CGNAT
        ("100.64.0.1", true), // CGNAT: CPython treats 100.64/10 as public
        ("192.0.0.8", false), // 192.0.0/24 IS private in 3.14 (only .9/.10 excepted)
        ("198.17.255.255", true),
        ("198.18.0.0", false),      // benchmarking /15 edge
        ("239.255.255.255", false), // multicast block end (pins >>28 mask)
        ("172.15.0.1", true),
        ("198.17.0.1", true),
        ("223.255.255.255", true),
        ("192.0.0.9", true), // private-range exceptions Python keeps
        ("192.0.0.10", true),
        ("2a01:4f8::1", true),
        ("::ffff:8.8.8.8", true), // mapped delegates to public v4
        // "drop the noise"
        ("10.1.2.3", false),
        ("127.0.0.1", false),
        ("169.254.5.5", false),
        ("172.16.0.1", false),
        ("172.31.255.255", false),
        ("192.0.0.1", false),
        ("192.0.2.1", false),
        ("192.168.1.1", false),
        ("198.18.0.1", false),
        ("198.19.255.255", false),
        ("198.51.100.1", false),
        ("203.0.113.9", false),
        ("224.0.0.1", false), // multicast
        ("240.0.0.1", false),
        ("255.255.255.254", false),
        ("0.0.0.0", false),
        ("::", false),
        ("::1", false),
        ("fe80::1", false),
        ("fc00::1", false),
        ("fd12::34", false),
        ("ff02::1", false),
        ("100::64", false),         // discard-only
        ("2001:db8::1", false),     // documentation
        ("2001:2::1", false),       // benchmarking
        ("::ffff:10.0.0.1", false), // mapped to private v4
        ("::ffff:127.0.0.1", false),
        // hostnames feed cox-style entries; junk must die
        ("wsip-184-178-172-5.rn.hr.cox.net", true),
        ("proxy.Example.COM", true),
        ("localhost", false),
        ("1.2.3", true), // >=2 alnum labels: passes the hostname gate (Python did too)
        ("not host!", false),
    ];

    #[test]
    fn filter_matches_python_reference() {
        for (addr, keep) in VECTORS {
            let got = validate_host(addr).is_some();
            assert_eq!(got, *keep, "{addr}: expected keep={keep}");
        }
    }

    #[test]
    fn canonicalization() {
        assert_eq!(
            validate_host("2A01:04F8:0000:0000:0000:0000:0000:0001").as_deref(),
            Some("[2a01:4f8::1]")
        );
        assert_eq!(
            validate_host("[2a01:4f8::1]").as_deref(),
            Some("[2a01:4f8::1]")
        );
        assert_eq!(
            validate_host("008.008.008.008").as_deref(),
            Some("008.008.008.008")
        ); // not a valid literal IPv4 (std rejects leading zeros) but matches the
        // hostname gate — CPython's regex kept these too; parity preserved
        assert_eq!(validate_host("example."), None); // trailing dot strips to a single label
    }

    #[test]
    fn credentials_and_ports() {
        let p = make_proxy(Scheme::Http, "8.8.8.8", "80", Some("u"), None, "x").unwrap();
        assert_eq!(p.url(), "http://u:@8.8.8.8:80");
        assert!(make_proxy(Scheme::Http, "8.8.8.8", "0", None, None, "x").is_none());
        assert!(make_proxy(Scheme::Http, "8.8.8.8", "65536", None, None, "x").is_none());
        let no_cred =
            make_proxy(Scheme::Http, "8.8.8.8", "80", Some("bad user"), None, "x").unwrap();
        assert!(no_cred.user.is_none()); // junk credential dropped, entry kept (Python parity)
    }
}
