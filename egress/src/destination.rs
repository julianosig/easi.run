//! What a session's proxy admits as a destination name, and the bench profile's fixture host built on it: the proxy
//! parses requests and its `PROXY_BENCH_FIXTURE` with these, and the controller refuses at its start a
//! `CONTROLLER_BENCH_FIXTURE` that every session's proxy would refuse at its own.

use std::net::Ipv4Addr;

/// Validates a DNS name and returns it lower-cased without a trailing dot.
///
/// The last label must start with a letter, so numeric forms a system resolver might read as an address
/// (`2130706433`, `0x7f.1`) are refused before resolution; resolved addresses are classified regardless.
pub fn validate_name(text: &str) -> Option<String> {
    let name = text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase();
    if name.is_empty() || name.len() > 253 {
        return None;
    }
    let labels_ok = name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            && !label.starts_with('-')
            && !label.ends_with('-')
    });
    let last_ok = name.rsplit('.').next().and_then(|l| l.bytes().next()).is_some_and(|b| b.is_ascii_alphabetic());
    (labels_ok && last_ok).then_some(name)
}

/// The `bench` profile's single fixture host: this exact name maps to this exact address without DNS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchFixture {
    pub name: String,
    pub addr: Ipv4Addr,
}

impl BenchFixture {
    /// Parses `name=ipv4`. The address may be on a private network (that is the fixture's purpose) but never one
    /// that would reach the proxy itself or a non-unicast destination.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (name, addr) = spec.split_once('=').ok_or("expected name=ipv4")?;
        let name = validate_name(name).ok_or("invalid fixture host name")?;
        let addr: Ipv4Addr = addr.parse().map_err(|_| "invalid fixture IPv4 address")?;
        let [first, second, ..] = addr.octets();
        let refused = match (first, second) {
            (0, _) => Some("ipv4_this_network"),
            (127, _) => Some("ipv4_loopback"),
            (169, 254) => Some("ipv4_link_local"),
            (224..=239, _) => Some("ipv4_multicast"),
            (240.., _) => Some("ipv4_reserved"),
            _ => None,
        };
        match refused {
            Some(reason) => Err(format!("fixture address is not admissible ({reason})")),
            None => Ok(Self { name, addr }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_fixture_specs_are_strict() {
        let fixture = BenchFixture::parse("fixture.bench=10.20.0.5").unwrap();
        assert_eq!(fixture, BenchFixture { name: "fixture.bench".to_owned(), addr: "10.20.0.5".parse().unwrap() });
        assert_eq!(BenchFixture::parse("Fixture.Bench.=10.20.0.5").unwrap(), fixture);
        let long_label = format!("{}.bench=10.20.0.5", "a".repeat(63));
        let long_name = format!("{}.bench=10.20.0.5", ["a".repeat(61).as_str(); 4].join("."));
        for good in [
            "fixture.bench=10.20.0.5",
            "Fixture.Bench=10.20.0.5",
            "fixture.bench.=10.20.0.5",
            "takeover.play=10.251.0.10",
            "home_1.play=172.31.250.10",
            "pub.bench=8.8.4.4",
            "bench=192.168.1.1",
            "edge.bench=223.255.255.255",
            &long_label,
            &long_name,
        ] {
            assert!(BenchFixture::parse(good).is_ok(), "{good:?} refused");
        }
        let too_long_label = format!("{}.bench=10.20.0.5", "a".repeat(64));
        let too_long_name = format!("{}.bench=10.20.0.5", ["a".repeat(62).as_str(); 4].join("."));
        for bad in [
            "",
            "fixture.bench",
            "=10.20.0.5",
            "fixture.bench=",
            "fixture.bench=10.20.0",
            "fixture.bench=::1",
            "fixture.bench=127.0.0.1",
            "fixture.bench=169.254.169.254",
            "fixture.bench=0.0.0.0",
            "fixture.bench=224.0.0.1",
            "fixture.bench=239.255.255.255",
            "fixture.bench=255.255.255.255",
            "fixture.bench=10.20.0.5,10.20.0.6",
            "bad name=10.20.0.5",
            "sites.1=10.251.0.10",
            "-sites=10.251.0.10",
            "sites-=10.251.0.10",
            "a..b=10.251.0.10",
            ".=10.251.0.10",
            "sites=127.0.0.1",
            "sites=240.0.0.1",
            "sit\u{e9}s=10.251.0.10",
            &too_long_label,
            &too_long_name,
        ] {
            assert!(BenchFixture::parse(bad).is_err(), "{bad:?} accepted");
        }
    }
}
