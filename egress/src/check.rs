//! What the check of an upstream found (the proxy's check mode, `proxy/src/check.rs`), as the one JSON line the check
//! prints, the controller holds to its shape and passes on, and the admin reads again:
//! `{"ok":true,"latency_ms":N,"ip":IP,"country":CC,"region":R?,"city":C?}`, or `{"ok":false,"reason":R,"detail":D}`.
//! Anything else is refused: another field, a country that is not two capitals or digits, a region or city that is not
//! a place name (see [`place`]), or a detail that is not 1 to [`MAX_DETAIL`] bytes of printable ASCII.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::{country_code, place};

/// The longest detail a failed check carries.
pub const MAX_DETAIL: usize = 200;

/// What the controller's refusal of a check says when as many checks as it runs at once are under way, which the admin
/// answers as busy (503) rather than as a failure.
pub const CHECKS_BUSY: &str = "egress checks are under way";

/// Why an upstream failed its check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckReason {
    /// Its name does not resolve, or nothing answers at its address and port.
    Unreachable,
    /// A step did not finish within the deadline.
    Timeout,
    /// It asks for credentials, or refused those given.
    Auth,
    /// It refused a tunnel, closed one before the far end answered, or names did not resolve through it to addresses
    /// sessions may reach.
    Refused,
    /// The TLS to an `https` upstream, or to the far end through a tunnel, did not verify or failed.
    Tls,
    /// It does not answer as the proxy its scheme names.
    Protocol,
    /// The speed test answered something else than its metadata.
    Unexpected,
}

/// Where the upstream's traffic leaves from: its address, that address's country, and its region and city when known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub ip: IpAddr,
    pub country: String,
    pub region: Option<String>,
    pub city: Option<String>,
}

/// How a check went, as its line says it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "Line", try_from = "Line")]
pub enum CheckOutcome {
    /// The speed test's metadata answered: the address it saw the request come from, and where that address is.
    Passed {
        latency_ms: u64,
        exit: Exit,
    },
    Failed {
        reason: CheckReason,
        detail: String,
    },
}

/// The line's fields, every one but `ok` present in one outcome only.
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ip: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<CheckReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl From<CheckOutcome> for Line {
    fn from(outcome: CheckOutcome) -> Line {
        match outcome {
            CheckOutcome::Passed { latency_ms, exit: Exit { ip, country, region, city } } => Line {
                ok: true,
                latency_ms: Some(latency_ms),
                ip: Some(ip),
                country: Some(country),
                region,
                city,
                ..Line::default()
            },
            CheckOutcome::Failed { reason, detail } => {
                Line { reason: Some(reason), detail: Some(detail), ..Line::default() }
            }
        }
    }
}

impl TryFrom<Line> for CheckOutcome {
    type Error = &'static str;

    fn try_from(line: Line) -> Result<CheckOutcome, &'static str> {
        let printable = |d: &str| (1..=MAX_DETAIL).contains(&d.len()) && d.bytes().all(|b| (b' '..=b'~').contains(&b));
        match line {
            Line {
                ok: true,
                latency_ms: Some(latency_ms),
                ip: Some(ip),
                country: Some(country),
                region,
                city,
                reason: None,
                detail: None,
            } if country_code(&country) && [&region, &city].iter().all(|p| p.as_deref().is_none_or(place)) => {
                Ok(CheckOutcome::Passed { latency_ms, exit: Exit { ip, country, region, city } })
            }
            Line {
                ok: false,
                latency_ms: None,
                ip: None,
                country: None,
                region: None,
                city: None,
                reason: Some(reason),
                detail: Some(detail),
            } if printable(&detail) => Ok(CheckOutcome::Failed { reason, detail }),
            _ => Err("not a check's outcome"),
        }
    }
}
