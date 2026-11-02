//! Notifications: when a person is needed, the admin POSTs an event to the administrator's webhook, if they set one, so
//! they hear of it wherever they are (a chat, a phone, their own tooling) without watching the admin's page.
//!
//! Three events, each naming what a person is to do and nothing a page showed: `person.needed` (an agent asked for a
//! person, see `person`: its reason and the message it wrote), `approval.needed` (an agent's file waits for the
//! administrator's answer, see `agent_files`: its kind, never the file's name, which can quote what a person typed),
//! and `unlock.needed` (an agent tried to resume a saved session while they are locked, see `profiles`). A test sends
//! `test`. Each carries `text` and `content`, one sentence in English, which Slack's and Discord's incoming webhooks
//! show as they are.
//!
//! The webhook is one `https://` URL (at most [`MAX_URL`] bytes) and the secret the admin made for it, kept in
//! `notifications.json` in the state directory (mode 0600, replaced atomically). Every POST is signed: header
//! `easi-run-signature: t=<Unix seconds>,v1=<hex HMAC-SHA256 of "<t>.<body>" with the secret>`. A POST waits at most
//! [`TIMEOUT`] (its answer's status only is read); one that fails or answers 5xx is tried again twice, after 2 and then
//! 10 seconds, unless the webhook was removed or replaced meanwhile. At most [`IN_FLIGHT`]
//! are under way at once; an event past that is dropped (and logged). The demo sends none.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

const FILE: &str = "notifications.json";
/// The longest webhook URL, in bytes.
pub const MAX_URL: usize = 2048;
/// How long one POST may take, connection included.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The waits before the second and the third try.
const RETRIES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];
/// POSTs under way at once, at most.
const IN_FLIGHT: usize = 8;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct Hook {
    url: String,
    secret: String,
}

type Https = hyper_rustls::HttpsConnector<HttpConnector>;

pub struct Notifier {
    hook: Arc<Mutex<Option<Hook>>>,
    path: PathBuf,
    /// Off in the demo.
    on: bool,
    sending: Arc<Semaphore>,
    client: Client<Https, Full<Bytes>>,
}

/// What a person is to do.
pub enum Event<'a> {
    /// An agent asked for a person: why (`person`'s reasons) and what it wrote.
    PersonNeeded {
        reason: &'a str,
        message: &'a str,
    },
    /// An agent's file waits for the administrator's answer: `download` or `transfer`.
    ApprovalNeeded {
        kind: &'a str,
    },
    /// An agent tried to resume a saved session while they are locked.
    UnlockNeeded,
    Test,
}

/// The session an event is about, as it names it.
pub struct About<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub watch_url: &'a str,
}

impl Notifier {
    /// The webhook the state directory keeps, if any; one that cannot be read is none, and said so.
    pub fn load(state_dir: &Path, demo: bool) -> Notifier {
        let path = state_dir.join(FILE);
        let hook = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Hook>(&bytes)
                .map_err(|why| tracing::warn!(%why, "notifications.json is not a webhook; none is set"))
                .ok()
                .filter(|hook| valid_url(&hook.url).is_ok()),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => None,
            Err(why) => {
                tracing::warn!(%why, "notifications.json could not be read; no webhook is set");
                None
            }
        };
        let https = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_only().enable_http1().build();
        Notifier {
            hook: Arc::new(Mutex::new(hook)),
            path,
            on: !demo,
            sending: Arc::new(Semaphore::new(IN_FLIGHT)),
            client: Client::builder(TokioExecutor::new()).build(https),
        }
    }

    /// The webhook's URL, if one is set.
    pub fn url(&self) -> Option<String> {
        self.hook.lock().unwrap().as_ref().map(|h| h.url.clone())
    }

    /// Sets the webhook to `url` with a new secret, which it returns (it is shown once).
    pub fn set(&self, url: &str) -> Result<String, String> {
        valid_url(url)?;
        let mut secret = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut secret)
            .map_err(|_| "no randomness for the secret".to_owned())?;
        let hook = Hook { url: url.to_owned(), secret: hex(&secret) };
        let bytes = serde_json::to_vec(&hook).map_err(|e| e.to_string())?;
        let mut held = self.hook.lock().unwrap();
        crate::private::replace(&self.path, &bytes).map_err(|why| why.to_string())?;
        *held = Some(hook.clone());
        Ok(hook.secret)
    }

    /// Removes the webhook.
    pub fn remove(&self) -> Result<(), String> {
        let mut held = self.hook.lock().unwrap();
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => return Err(format!("{}: {why}", self.path.display())),
        }
        *held = None;
        Ok(())
    }

    /// Sends `event` about `about` to the webhook, if one is set, on its own task (retried as the module says).
    pub fn send(&self, event: Event, about: Option<About>) {
        let Some(hook) = self.hook.lock().unwrap().clone().filter(|_| self.on) else { return };
        let Ok(permit) = self.sending.clone().try_acquire_owned() else {
            tracing::warn!("a notification was dropped: too many are under way");
            return;
        };
        let body = payload(&event, about.as_ref(), crate::timeline::now_ms());
        let (client, current) = (self.client.clone(), self.hook.clone());
        tokio::spawn(async move {
            let _permit = permit;
            for wait in std::iter::once(None).chain(RETRIES.map(Some)) {
                if let Some(wait) = wait {
                    tokio::time::sleep(wait).await;
                    // A webhook removed or replaced meanwhile gets nothing more.
                    if current.lock().unwrap().as_ref() != Some(&hook) {
                        return;
                    }
                }
                match post(&client, &hook, &body).await {
                    Ok(status) if status < 500 => return,
                    Ok(status) => tracing::warn!(status, "the webhook answered with a server error"),
                    Err(why) => tracing::warn!(%why, "the webhook could not be reached"),
                }
            }
        });
    }

    /// Sends a `test` event now, once, and says how the webhook answered: its status, or why it could not be reached.
    pub async fn test(&self) -> Result<u16, String> {
        let hook = self.hook.lock().unwrap().clone().ok_or("no webhook is set")?;
        let body = payload(&Event::Test, None, crate::timeline::now_ms());
        post(&self.client, &hook, &body).await
    }
}

/// Whether `url` can be a webhook: `https://`, a host, no user or password, at most [`MAX_URL`] bytes.
pub fn valid_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "the webhook must be an https:// URL".to_owned())?;
    if url.len() > MAX_URL {
        return Err(format!("the webhook URL is at most {MAX_URL} bytes"));
    }
    if parsed.scheme() != "https"
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("the webhook must be an https:// URL with a host and no user name or password".into());
    }
    Ok(())
}

/// The JSON an event is POSTed as.
fn payload(event: &Event, about: Option<&About>, at: u64) -> Vec<u8> {
    let (name, mut body, said) = match event {
        Event::PersonNeeded { reason, message } => {
            let what = match *reason {
                "login" => "to log in",
                "captcha" => "to solve a challenge",
                "confirm" => "to confirm something",
                _ => "for help",
            };
            let said = if message.is_empty() { String::new() } else { format!(": “{message}”") };
            ("person.needed", json!({"reason": reason, "message": message}), format!("The agent asks you {what}{said}"))
        }
        Event::ApprovalNeeded { kind } => {
            let what = if *kind == "download" { "save a download" } else { "send a file into the browser" };
            ("approval.needed", json!({"kind": kind}), format!("The agent asks to {what}; answer in the session"))
        }
        Event::UnlockNeeded => (
            "unlock.needed",
            json!({}),
            "An agent tried to resume a saved session, but they are locked: log in to easi.run with your password"
                .to_owned(),
        ),
        Event::Test => ("test", json!({}), "A test notification from easi.run".to_owned()),
    };
    let text = match about {
        Some(about) => format!("easi.run · {}: {said}. {}", about.label, about.watch_url),
        None => format!("easi.run: {said}."),
    };
    body["event"] = json!(name);
    body["at"] = json!(at);
    body["text"] = json!(text);
    body["content"] = json!(text);
    if let Some(about) = about {
        body["session"] = json!({"id": about.id, "label": about.label, "watch_url": about.watch_url});
    }
    serde_json::to_vec(&body).unwrap_or_default()
}

/// `t=<seconds>,v1=<hex HMAC-SHA256 of "<t>.<body>">`.
fn signature(secret: &str, at_seconds: u64, body: &[u8]) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    let mut signed = format!("{at_seconds}.").into_bytes();
    signed.extend_from_slice(body);
    format!("t={at_seconds},v1={}", hex(ring::hmac::sign(&key, &signed).as_ref()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One POST of `body` to the webhook, within [`TIMEOUT`]: its status, or why it was not answered.
async fn post(client: &Client<Https, Full<Bytes>>, hook: &Hook, body: &[u8]) -> Result<u16, String> {
    let at = crate::timeline::now_ms() / 1000;
    let request = hyper::Request::post(&hook.url)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::USER_AGENT, "easi.run")
        .header("easi-run-signature", signature(&hook.secret, at, body))
        .body(Full::new(Bytes::from(body.to_vec())))
        .map_err(|e| e.to_string())?;
    let answered = tokio::time::timeout(TIMEOUT, async {
        // Its status only: what it says is never read (dropped with its connection).
        let response = client.request(request).await.map_err(|e| e.to_string())?;
        Ok::<u16, String>(response.status().as_u16())
    })
    .await;
    answered.map_err(|_| format!("no answer within {} s", TIMEOUT.as_secs()))?
}

/// What `GET /api/notifications` answers.
pub fn shown(notifier: &Notifier) -> Value {
    json!({"url": notifier.url()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_https_url_with_a_host_and_no_credentials_is_a_webhook() {
        assert!(valid_url("https://hooks.example.com/abc?x=1").is_ok());
        for bad in ["http://hooks.example.com/", "https://user:pw@example.com/", "ftp://x/", "not a url"] {
            assert!(valid_url(bad).is_err(), "{bad}");
        }
        assert!(valid_url(&format!("https://example.com/{}", "a".repeat(MAX_URL))).is_err());
    }

    #[test]
    fn an_event_says_what_a_person_is_to_do_and_where_and_is_signed_over_its_time_and_body() {
        let about = About { id: "abc", label: "Bank", watch_url: "https://easi.example/#/s/abc" };
        let body = payload(&Event::PersonNeeded { reason: "login", message: "Sign in to the bank" }, Some(&about), 7);
        let event: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(event["event"], "person.needed");
        assert_eq!(event["session"]["watch_url"], "https://easi.example/#/s/abc");
        assert_eq!(event["text"], event["content"]);
        let text = event["text"].as_str().unwrap();
        assert!(
            text.contains("to log in") && text.contains("Sign in to the bank") && text.contains("#/s/abc"),
            "{text}"
        );
        let approval: Value =
            serde_json::from_slice(&payload(&Event::ApprovalNeeded { kind: "download" }, None, 7)).unwrap();
        assert_eq!((&approval["event"], &approval["kind"]), (&json!("approval.needed"), &json!("download")));
        // The receiver's check: the same HMAC over "<t>.<body>".
        let signed = signature("s3cret", 1_700_000_000, &body);
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"s3cret");
        let mut message = b"1700000000.".to_vec();
        message.extend_from_slice(&body);
        let expected = hex(ring::hmac::sign(&key, &message).as_ref());
        assert_eq!(signed, format!("t=1700000000,v1={expected}"));
    }

    #[test]
    fn a_webhook_is_kept_with_a_new_secret_and_removed() {
        let dir = crate::private::test_dir("notify");
        let notifier = Notifier::load(&dir, false);
        assert_eq!(notifier.url(), None);
        let secret = notifier.set("https://hooks.example.com/a").unwrap();
        assert_eq!(secret.len(), 64);
        assert_ne!(notifier.set("https://hooks.example.com/a").unwrap(), secret, "a new secret each time");
        assert!(notifier.set("http://hooks.example.com/a").is_err());
        let reloaded = Notifier::load(&dir, false);
        assert_eq!(reloaded.url().as_deref(), Some("https://hooks.example.com/a"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        reloaded.remove().unwrap();
        assert_eq!(Notifier::load(&dir, false).url(), None);
    }
}
