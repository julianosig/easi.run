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

/// The outcome of one request, serialized as its log line's fields in their order.
#[derive(Default, Serialize)]
pub struct Record {
    pub client: Option<SocketAddr>,
    #[serde(serialize_with = "capped")]
    pub method: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub resolved: Option<IpAddr>,
    #[serde(rename = "result", serialize_with = "result")]
    pub allowed: bool,
    pub reason: &'static str,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub duration_ms: u128,
}

/// One log line: its time, then the record.
#[derive(Serialize)]
struct Line<'a> {
    ts: String,
    #[serde(flatten)]
    record: &'a Record,
}

impl Record {
    pub fn to_json(&self) -> String {
        serde_json::to_string(&Line { ts: timestamp(SystemTime::now()), record: self }).unwrap_or_default()
    }
}

/// A method is a token from the head; bound what a hostile client can put in the log.
fn capped<S: serde::Serializer>(method: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    method.as_deref().map(|m| m.get(..32).unwrap_or(m)).serialize(s)
}

fn result<S: serde::Serializer>(allowed: &bool, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(if *allowed { "allowed" } else { "refused" })
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

    #[test]
    fn a_line_keeps_its_fields_in_order_with_the_method_capped() {
        let record = Record { method: Some("M".repeat(40)), allowed: true, reason: "ok", ..Record::default() };
        let line = record.to_json();
        let (_, fields) = line.split_once("Z\",").unwrap();
        let method = "M".repeat(32);
        assert_eq!(
            fields,
            format!(
                r#""client":null,"method":"{method}","host":null,"port":null,"resolved":null,"result":"allowed","reason":"ok","bytes_up":0,"bytes_down":0,"duration_ms":0}}"#
            )
        );
    }
}
