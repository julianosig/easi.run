//! The closed request schema: one JSON object per line, `{"op": ...}`, no unknown operation or field.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::template::Spec;

pub const MAX_URL: usize = 2048;

/// Twelve lowercase hex characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct SessionId(String);

impl FromStr for SessionId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if crate::image::hex(text, 12) {
            Ok(Self(text.to_owned()))
        } else {
            Err("a session id is 12 lowercase hex characters".into())
        }
    }
}

impl TryFrom<String> for SessionId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub session: SessionId,
    pub url: String,
    pub devtools: bool,
    pub width: u32,
    pub height: u32,
    /// The browser's time zone (an IANA name) and languages (BCP 47 tags, preferred first), so a page sees them agree
    /// with where its traffic comes from.
    pub timezone: String,
    pub languages: Vec<String>,
    /// A benchmark lane (the bench profile only; see `template::Lane`).
    #[serde(default)]
    pub lane: Option<u8>,
    /// The upstream proxy the session's traffic leaves through, if the deployment configured one for it.
    #[serde(default)]
    pub egress: Option<egress::Upstream>,
    /// Chrome in kiosk mode, the page alone on the screen as on the KasmVNC baseline (the bench profile only).
    #[serde(default)]
    pub kiosk: bool,
    /// The admin requires the walled garden (the public demo): refused unless this controller enforces it.
    #[serde(default)]
    pub walled: bool,
    /// The session starts from a saved profile: its browser waits for the admin to restore it (`profile_restore`)
    /// before Chrome starts, and the admin saves it (`profile_save`) when the session ends.
    #[serde(default)]
    pub profile: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub session: SessionId,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

/// An upstream to check (`check_egress`), as a session would leave through it: its `{session}` placeholder already
/// filled.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub upstream: egress::Upstream,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    CreateSession(Create),
    StopSession(Session),
    InspectSession(Session),
    AgentAttach(Session),
    AgentTerminate(Session),
    AgentRestart(Session),
    ProfileRestore(Session),
    ProfileSave(Session),
    CheckEgress(Check),
    Cleanup(Empty),
}

impl Request {
    pub fn parse(line: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(line).map_err(|e| e.to_string())
    }

    pub fn name(&self) -> &'static str {
        match self {
            Request::CreateSession(_) => "create_session",
            Request::StopSession(_) => "stop_session",
            Request::InspectSession(_) => "inspect_session",
            Request::AgentAttach(_) => "agent_attach",
            Request::AgentTerminate(_) => "agent_terminate",
            Request::AgentRestart(_) => "agent_restart",
            Request::ProfileRestore(_) => "profile_restore",
            Request::ProfileSave(_) => "profile_save",
            Request::CheckEgress(_) => "check_egress",
            Request::Cleanup(_) => "cleanup",
        }
    }
}

impl Create {
    /// The URL, size, and id checks of `create_session`.
    pub fn validate(self) -> Result<Spec, String> {
        let rest = self.url.strip_prefix("https://").or_else(|| self.url.strip_prefix("http://"));
        match rest {
            None => return Err("url must start with https:// or http://".into()),
            Some(rest) if rest.is_empty() || rest.starts_with('/') => return Err("url has no host".into()),
            Some(_) => {}
        }
        if self.url.len() > MAX_URL {
            return Err(format!("url is longer than {MAX_URL} bytes"));
        }
        if self.url.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("url contains whitespace or a control character".into());
        }
        // The rule the admin checks its ADMIN_OUTPUT and every size asked for by.
        if !display_protocol::frame::output_size_ok(self.width, self.height) {
            return Err(format!("size must be {}", display_protocol::frame::OUTPUT_SIZE_RULE));
        }
        if !locale::valid_timezone(&self.timezone) {
            return Err("timezone must be an IANA time zone such as America/Sao_Paulo".into());
        }
        if !locale::valid_languages(&self.languages) {
            return Err(format!("languages must be 1 to {} tags such as pt-BR", locale::MAX_LANGUAGES));
        }
        if let Some(Err(why)) = self.egress.as_ref().map(egress::Upstream::validate) {
            return Err(format!("egress: {why}"));
        }
        Ok(Spec {
            session: self.session,
            url: self.url,
            devtools: self.devtools,
            width: self.width,
            height: self.height,
            timezone: self.timezone,
            languages: self.languages,
            bench_spki: None,
            lane: self.lane,
            egress: self.egress,
            kiosk: self.kiosk,
            walled: self.walled,
            profile: self.profile,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid create_session request with `fields` replacing its own, parsed and validated.
    /// A valid creation request.
    fn valid_create() -> serde_json::Value {
        serde_json::json!({
            "op": "create_session", "session": "0123456789ab", "url": "https://example.com/", "devtools": false,
            "width": 1280, "height": 800, "timezone": "America/Sao_Paulo", "languages": ["pt-BR", "en"],
        })
    }

    fn create_with(fields: serde_json::Value) -> Result<Spec, String> {
        let mut line = valid_create();
        line.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        match Request::parse(line.to_string().as_bytes())? {
            Request::CreateSession(create) => create.validate(),
            other => panic!("{other:?}"),
        }
    }

    fn create(url: &str, width: u32, height: u32) -> Result<Spec, String> {
        create_with(serde_json::json!({ "url": url, "width": width, "height": height }))
    }

    #[test]
    fn every_operation_parses() {
        for op in [
            "stop_session",
            "inspect_session",
            "agent_attach",
            "agent_terminate",
            "agent_restart",
            "profile_restore",
            "profile_save",
        ] {
            let request = Request::parse(format!(r#"{{"op":"{op}","session":"0123456789ab"}}"#).as_bytes()).unwrap();
            assert_eq!(request.name(), op);
        }
        assert_eq!(Request::parse(br#"{"op":"cleanup"}"#).unwrap().name(), "cleanup");
        let check = r#"{"op":"check_egress","upstream":{"scheme":"http","host":"proxy.example.net","port":8080}}"#;
        assert_eq!(Request::parse(check.as_bytes()).unwrap().name(), "check_egress");
        assert!(create("https://example.com/", 1280, 800).is_ok());
    }

    #[test]
    fn unknown_operations_and_fields_are_refused() {
        for line in [
            r#"{"op":"exec","session":"0123456789ab","cmd":["sh"]}"#,
            r#"{"op":"cleanup","all":true}"#,
            r#"{"op":"stop_session","session":"0123456789ab","force":true}"#,
            r#"{"op":"agent_attach","session":"0123456789ab","user":"0"}"#,
            r#"{"op":"profile_restore","session":"0123456789ab","path":"/etc"}"#,
            r#"{"op":"profile_save","session":"0123456789ab","cmd":["sh"]}"#,
            r#"{"op":"profile_save"}"#,
            r#"{"op":"check_egress"}"#,
            r#"{"op":"check_egress","upstream":{"scheme":"http","host":"p.example","port":8080},"image":"x"}"#,
            r#"{"op":"check_egress","upstream":{"scheme":"http","host":"p.example","port":8080,"cmd":["sh"]}}"#,
            r#"{"op":"check_egress","session":"0123456789ab","upstream":{"scheme":"http","host":"p.example","port":1}}"#,
            r#"{"session":"0123456789ab"}"#,
            r#"{"op":"stop_session","session":"0123456789ab","session":"0123456789ac"}"#,
            r#"{"op":"stop_session"}"#,
            r#"["stop_session"]"#,
            "",
            "not json",
        ] {
            assert!(Request::parse(line.as_bytes()).is_err(), "{line}");
        }
        // A creation wrong in exactly one way: an unknown field, a missing one, or one of the wrong type.
        assert!(Request::parse(valid_create().to_string().as_bytes()).is_ok());
        let mut missing = valid_create();
        missing.as_object_mut().unwrap().remove("devtools");
        let wrong = [("privileged", serde_json::json!(true)), ("devtools", "yes".into()), ("width", 1280.5.into())];
        for line in wrong
            .into_iter()
            .map(|(field, value)| {
                let mut line = valid_create();
                line[field] = value;
                line
            })
            .chain([missing])
        {
            assert!(Request::parse(line.to_string().as_bytes()).is_err(), "{line}");
        }
    }

    #[test]
    fn a_sessions_time_zone_and_languages_are_checked() {
        let place = |timezone: &str, languages| {
            create_with(serde_json::json!({ "timezone": timezone, "languages": languages })).map(|_| ())
        };
        assert!(place("Europe/London", serde_json::json!(["en-GB"])).is_ok());
        assert!(place("America/DefinitelyNotAZone", serde_json::json!(["en"])).is_err());
        assert!(place("UTC", serde_json::json!([])).is_err());
        assert!(place("UTC", serde_json::json!(["en;q=0.9"])).is_err());
    }

    #[test]
    fn a_sessions_upstream_is_checked() {
        let with = |egress| create_with(serde_json::json!({ "egress": egress })).map(|spec| spec.egress);
        let good = serde_json::json!({"scheme": "socks5", "host": "proxy.example.net", "port": 1080,
                                      "username": "u", "password": "p"});
        assert!(with(good).unwrap().is_some());
        for bad in [
            serde_json::json!({"scheme": "ftp", "host": "proxy.example.net", "port": 21}),
            serde_json::json!({"scheme": "http", "host": "proxy example", "port": 8080}),
            serde_json::json!({"scheme": "http", "host": "proxy.example.net", "port": 0}),
            serde_json::json!({"scheme": "socks5", "host": "proxy.example.net", "port": 1080, "username": "u"}),
            serde_json::json!({"scheme": "http", "host": "proxy.example.net", "port": 8080, "extra": 1}),
        ] {
            assert!(with(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn session_ids() {
        for id in ["0123456789ab", "ffffffffffff"] {
            assert!(id.parse::<SessionId>().is_ok());
        }
        for id in ["0123456789AB", "0123456789a", "0123456789abc", "0123456789ag", "../../etc/x", ""] {
            assert!(id.parse::<SessionId>().is_err(), "{id}");
        }
        assert!(Request::parse(br#"{"op":"stop_session","session":"0123456789AB"}"#).is_err());
    }

    #[test]
    fn urls_and_sizes() {
        assert!(create("http://example.com", 320, 240).is_ok());
        assert!(create("https://example.com", 1920, 1080).is_ok());
        assert!(create(&format!("https://e.test/{}", "a".repeat(MAX_URL - 15)), 1280, 800).is_ok());
        assert!(create(&format!("https://e.test/{}", "a".repeat(MAX_URL - 14)), 1280, 800).is_err());
        for url in [
            "file:///etc/passwd",
            "chrome://settings",
            "javascript:alert(1)",
            "HTTPS://e.test",
            "https://",
            "https:///x",
            "https://e.test/a b",
            "https://e.test/\n",
            "ftp://e.test",
        ] {
            assert!(create(url, 1280, 800).is_err(), "{url}");
        }
        for (w, h) in [(318, 240), (320, 238), (1922, 1080), (1920, 1082), (1281, 800), (1280, 801), (1488, 962)] {
            assert!(create("https://e.test", w, h).is_err(), "{w}x{h}");
        }
    }
}
