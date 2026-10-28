//! What the admin's answers name a refusal or failure by, for the administrator's page to say in its own language: a
//! stable code, the English it is told in (for agents, the agent API and logs, which read it as it is), and the values
//! the page's message needs (a limit, a name), as fields of their own, never inside English.
//!
//! Every answer that can reach the page carries one: `{"code", "error", ...fields}` for a refusal, and the same object
//! where a successful answer holds a failure (a session's `failed`, a saved session's `unsaved`). An opaque diagnostic
//! (the controller's, Docker's, a file's) has a generic code, its English kept in `error`.

use serde_json::{Map, Value, json};

macro_rules! codes {
    ($($(#[$doc:meta])* $variant:ident = $text:literal,)*) => {
        /// A refusal's or failure's code (see the module's notes).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Code {
            $($(#[$doc])* $variant,)*
        }

        impl Code {
            /// Every code: the page has a message for each (client/messages/en/errors.json), which a test holds it to.
            #[cfg(test)]
            pub const ALL: &[Code] = &[$(Code::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Code::$variant => $text,)*
                }
            }
        }
    };
}

codes! {
    // Logging in, and what a login allows.
    LogIn = "log-in",
    Forbidden = "forbidden",
    /// With `origin`, the address easi.run is set up for.
    WrongOrigin = "wrong-origin",
    TooManyAttempts = "too-many-attempts",
    /// The step after the password lapsed: the password is asked again.
    LoginAgain = "login-again",
    WrongPassword = "wrong-password",
    /// With `min` and `max`.
    PasswordLength = "password-length",
    /// A request the page never makes (malformed, or missing what it needs).
    BadRequest = "bad-request",
    /// Something on the server failed; `error` says what, in English.
    ServerFailed = "server-failed",
    /// What was asked could not be written on the server, so nothing changed; its log says why.
    ServerUnsaved = "server-unsaved",

    // The second step at sign-in.
    PasskeyNeedsDomain = "passkey-needs-domain",
    /// With `max`.
    TooManyPasskeys = "too-many-passkeys",
    PasskeyExpired = "passkey-expired",
    PasskeyRefused = "passkey-refused",
    PasskeyAddedAlready = "passkey-added-already",
    NoPasskey = "no-passkey",
    AppSetupExpired = "app-setup-expired",
    AppNeedsPassword = "app-needs-password",
    AppCodeWrong = "wrong-code",
    AppOff = "app-off",
    SecondStepOff = "second-step-off",

    // The public demo.
    DemoUnknown = "demo-unknown",
    DemoOpening = "demo-opening",
    DemoFull = "demo-full",
    DemoYours = "demo-yours",
    DemoOften = "demo-often",
    DemoTurnOver = "demo-turn-over",
    DemoSwitching = "demo-switching",
    DemoSwitchNeedsTurn = "demo-switch-needs-turn",
    DemoNotOpened = "demo-not-opened",
    VisitOver = "visit-over",
    DemoOpenElsewhere = "demo-open-elsewhere",
    NotInDemo = "not-in-demo",

    // Sessions, their control and their agent.
    NoSession = "no-session",
    SessionFailed = "session-failed",
    ControlHeld = "control-held",
    NoViewer = "no-viewer",
    ViewerLeft = "viewer-left",
    ControlNotHeld = "control-not-held",
    PersonInControl = "person-in-control",
    AgentBusy = "agent-busy",
    TooManySessions = "too-many-sessions",
    /// With `max`, in bytes.
    BadUrl = "bad-url",
    BadSize = "bad-size",
    DisplayNotStarted = "display-not-started",
    StopFailed = "stop-failed",
    TokenRevoked = "token-revoked",
    NoDownload = "no-download",
    DownloadsBusy = "downloads-busy",
    ControlMoved = "control-moved",
    DownloadGone = "download-gone",

    // Agent tokens.
    /// With `max`.
    TokenName = "token-name",
    /// With `max`.
    TooManyTokens = "too-many-tokens",
    NoToken = "no-token",
    /// Revoked, so refused now, but not yet written: a restart before the next write would accept it again.
    TokenRevokeUnsaved = "token-revoke-unsaved",
    /// Revoked, but stopping its sessions failed.
    TokenSessionsNotStopped = "token-sessions-not-stopped",

    // Saved sessions.
    NoSavedSession = "no-saved-session",
    SavedLocked = "saved-locked",
    SavedOtherPassword = "saved-other-password",
    SavedRunning = "saved-running",
    /// With `max`.
    SavedName = "saved-name",
    /// With `max`.
    TooManySaved = "too-many-saved",
    SavedDataMissing = "saved-data-missing",
    SavedArchiveBroken = "saved-archive-broken",
    SavedNotRestored = "saved-not-restored",
    /// With `max`, in MiB.
    SaveTooLarge = "save-too-large",
    SaveFailed = "save-failed",

    // Proxies (egresses), and their checks.
    NoProxy = "no-proxy",
    /// With `max`.
    ProxyName = "proxy-name",
    ProxyNameTaken = "proxy-name-taken",
    ProxyUrl = "proxy-url",
    ProxyTimezone = "proxy-timezone",
    /// With `max`.
    ProxyLanguages = "proxy-languages",
    /// With `max`.
    TooManyProxies = "too-many-proxies",
    /// With `sessions`, the ids of those that use it.
    ProxyInUse = "proxy-in-use",
    ProxyChecksBusy = "proxy-checks-busy",
    ProxyCheckUnavailable = "proxy-check-unavailable",
    ProxyCheckUnreachable = "proxy-check-unreachable",
    ProxyCheckTimeout = "proxy-check-timeout",
    ProxyCheckAuth = "proxy-check-auth",
    ProxyCheckRefused = "proxy-check-refused",
    ProxyCheckTls = "proxy-check-tls",
    ProxyCheckProtocol = "proxy-check-protocol",
    ProxyCheckUnexpected = "proxy-check-unexpected",
}

impl Code {
    /// A failed proxy check's code, by the reason the controller gave it (see `egresses::REASONS`).
    pub fn of_check(reason: &str) -> Code {
        match reason {
            "timeout" => Code::ProxyCheckTimeout,
            "auth" => Code::ProxyCheckAuth,
            "refused" => Code::ProxyCheckRefused,
            "tls" => Code::ProxyCheckTls,
            "protocol" => Code::ProxyCheckProtocol,
            "unexpected" => Code::ProxyCheckUnexpected,
            _ => Code::ProxyCheckUnreachable,
        }
    }
}

/// A refusal or failure: its code, its English, and the values its message needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Coded {
    pub code: Code,
    pub why: String,
    fields: Map<String, Value>,
}

impl Coded {
    pub fn new(code: Code, why: impl Into<String>) -> Coded {
        Coded { code, why: why.into(), fields: Map::new() }
    }

    /// With the value `name` the page's message takes.
    pub fn with(mut self, name: &str, value: impl Into<Value>) -> Coded {
        self.fields.insert(name.to_owned(), value.into());
        self
    }

    /// The same, told in English otherwise (with more said, say).
    pub fn told(mut self, why: impl Into<String>) -> Coded {
        self.why = why.into();
        self
    }

    /// As the answers carry it: `{"code", "error", ...fields}`.
    pub fn json(&self) -> Value {
        let mut body = json!({"code": self.code.as_str(), "error": self.why});
        for (name, value) in &self.fields {
            body[name] = value.clone();
        }
        body
    }
}

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.why)
    }
}

impl serde::Serialize for Coded {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.json().serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_has_a_message_for_every_code_and_for_no_other() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/messages/en/errors.json");
        let messages: Map<String, Value> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut said: Vec<&str> = messages.keys().map(String::as_str).filter(|k| !k.starts_with("generic.")).collect();
        let mut codes: Vec<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
        said.sort_unstable();
        codes.sort_unstable();
        assert_eq!(said, codes);
    }

    #[test]
    fn a_coded_refusal_carries_its_code_english_and_values() {
        let coded = Coded::new(Code::TooManyTokens, "at most 32 tokens; revoke one first").with("max", 32);
        assert_eq!(
            coded.json(),
            json!({"code": "too-many-tokens", "error": "at most 32 tokens; revoke one first", "max": 32})
        );
        let codes: std::collections::HashSet<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(codes.len(), Code::ALL.len(), "each code once");
    }
}
