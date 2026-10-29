//! One JSON line per request. Paths, query strings, and headers are never recorded.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Where finished log lines go.
pub type Sink = Arc<dyn Fn(String) + Send + Sync>;

/// The stdout sink used in production.
pub fn stdout_sink() -> Sink {
    Arc::new(|line: String| {
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    })
}

/// The outcome of one request.
#[derive(Default)]
pub struct Record {
    pub client: Option<SocketAddr>,
    pub method: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub resolved: Option<IpAddr>,
    pub allowed: bool,
    pub reason: &'static str,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub duration_ms: u128,
}

/// One log line's fields, in their order.
#[derive(Serialize)]
struct Line<'a> {
    ts: String,
    client: Option<SocketAddr>,
    method: Option<&'a str>,
    host: Option<&'a str>,
    port: Option<u16>,
    resolved: Option<IpAddr>,
    result: &'static str,
    reason: &'static str,
    bytes_up: u64,
    bytes_down: u64,
    duration_ms: u128,
}

impl Record {
    pub fn to_json(&self) -> String {
        let line = Line {
            ts: timestamp(SystemTime::now()),
            client: self.client,
            // A method is a token from the head; bound what a hostile client can put in the log.
            method: self.method.as_deref().map(|m| m.get(..32).unwrap_or(m)),
            host: self.host.as_deref(),
            port: self.port,
            resolved: self.resolved,
            result: if self.allowed { "allowed" } else { "refused" },
            reason: self.reason,
            bytes_up: self.bytes_up,
            bytes_down: self.bytes_down,
            duration_ms: self.duration_ms,
        };
        serde_json::to_string(&line).unwrap_or_default()
    }
}

/// RFC 3339 UTC with milliseconds.
fn timestamp(now: SystemTime) -> String {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        since.subsec_millis()
    )
}

/// Howard Hinnant's days-to-civil conversion.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_are_rfc3339() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let leap = UNIX_EPOCH + Duration::from_millis(951_782_400_123); // 2000-02-29
        assert_eq!(timestamp(leap), "2000-02-29T00:00:00.123Z");
        let later = UNIX_EPOCH + Duration::from_secs(1_792_281_599); // 2026-10-17T23:59:59Z
        assert_eq!(timestamp(later), "2026-10-17T23:59:59.000Z");
    }

    #[test]
    fn strings_are_escaped() {
        let record = Record { method: Some("G\"E\\T\n".to_owned()), reason: "x", ..Record::default() };
        let line = record.to_json();
        assert!(line.contains(r#""method":"G\"E\\T\n""#), "{line}");
        assert!(line.starts_with("{\"ts\":\"") && line.ends_with("\"duration_ms\":0}"), "{line}");
    }
}
