//! A client of the running admin application for the end-to-end tests: its HTTPS API and its viewer WebSockets,
//! speaking the real wire protocol. The tests are ignored by default; scripts/e2e.sh runs them against a stack started
//! with the bench profile, which `ADMIN_ORIGIN` (default `https://localhost:9444`) and `ADMIN_PASSWORD` locate and
//! unlock. The suites run side by side against that one admin (each test stops only the sessions it started), except
//! the tests in a module named `exclusive`, which change the admin for everyone and run alone.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use display_protocol::{Direction, LeaseHolder, Message, VERSION};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

/// The first `Some` that `probe`, evaluated afresh each time, gives in at most `tries` tries `every` apart, or `None`.
/// A macro rather than a function, so the probe may await and may borrow or assign what is around it.
macro_rules! until {
    ($tries:expr, $every:expr, $probe:expr) => {{
        let mut found = None;
        for _ in 0..$tries {
            found = $probe;
            if found.is_some() {
                break;
            }
            tokio::time::sleep($every).await;
        }
        found
    }};
}
// Not every test file polls, as not every one uses every helper (dead_code above).
#[allow(unused_imports)]
pub(crate) use until;

/// The bench stack's names (compose.bench.yaml): its Compose project's admin and controller containers, and its
/// controller's instance, which starts every session resource's name (`<instance>-<id>-<role>`).
pub const ADMIN_CONTAINER: &str = "easi-run-bench-admin-1";
pub const CONTROLLER_CONTAINER: &str = "easi-run-bench-controller-1";
pub const INSTANCE: &str = "easirun-bench";

/// The admin's origin, which must be HTTPS on this host's loopback: the tests accept any certificate there (the
/// bench admin's is self-signed) and send it the admin password, which no other host may receive.
pub fn origin() -> String {
    let origin = std::env::var("ADMIN_ORIGIN").unwrap_or_else(|_| "https://localhost:9444".into());
    let url = reqwest::Url::parse(&origin).unwrap_or_else(|e| panic!("ADMIN_ORIGIN {origin:?}: {e}"));
    let local = url.host_str().is_some_and(|host| loopback(host.trim_start_matches('[').trim_end_matches(']')));
    assert!(
        url.scheme() == "https" && local,
        "ADMIN_ORIGIN {origin:?} is not https on localhost or a loopback address"
    );
    origin
}

fn loopback(host: &str) -> bool {
    host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Accepts the admin's self-signed certificate, from a loopback host only: these tests talk to a local stack they
/// started, and nothing they reach elsewhere (a redirect, say) is trusted.
#[derive(Debug)]
struct LocalAdmin;

impl rustls::client::danger::ServerCertVerifier for LocalAdmin {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        name: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let local = match name {
            rustls::pki_types::ServerName::DnsName(dns) => dns.as_ref() == "localhost",
            rustls::pki_types::ServerName::IpAddress(ip) => std::net::IpAddr::from(*ip).is_loopback(),
            _ => false,
        };
        if !local {
            return Err(rustls::Error::General(format!("{name:?} is not the local admin")));
        }
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

/// An HTTP client of the admin. It lets an idle connection go after 5 s: the admin closes one idle for 10 s
/// (admin/src/server.rs, `HEADERS`), and a request sent on it as the admin closes it fails ("error sending request").
fn client() -> reqwest::ClientBuilder {
    reqwest::Client::builder().use_preconfigured_tls(tls()).pool_idle_timeout(Duration::from_secs(5))
}

fn tls() -> rustls::ClientConfig {
    let _ = rustls::crypto::ring::default_provider().install_default();
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(LocalAdmin))
        .with_no_client_auth()
}

/// Fixtures more than one test file opens (bench/fixtures, served at `https://fixture.bench/`).
pub const TYPING: &str = "https://fixture.bench/typing.html";
pub const SAVE: &str = "https://fixture.bench/save.html";
pub const UPLOAD: &str = "https://fixture.bench/upload.html";
/// The typing fixture's text field, in output pixels.
pub const FIELD: (u16, u16) = (400, 62 + PAGE_TOP);

/// Where a page starts on the screen: below Chrome's tab strip and toolbar, in output pixels.
pub const PAGE_TOP: u16 = 90;

pub struct Admin {
    pub http: reqwest::Client,
    pub origin: String,
    pub csrf: String,
    pub cookie: String,
    /// The sessions this login started and has not stopped: the only ones its [`Admin::clear`] and its drop stop.
    own: Mutex<Vec<String>>,
}

#[derive(Debug)]
pub struct Refused(pub u16, pub String);

impl Admin {
    /// Logs in, or `None` while the admin is not answering.
    pub async fn try_login() -> Option<Admin> {
        let origin = origin();
        let http = client().build().ok()?;
        http.get(format!("{origin}/api/status")).send().await.ok()?.error_for_status().ok()?;
        Some(Admin::login().await)
    }

    pub async fn login() -> Admin {
        let http = client().build().expect("an HTTP client");
        Admin::open(http, origin()).await.unwrap_or_else(|why| panic!("{why}"))
    }

    /// Logs in with `ADMIN_PASSWORD` through `http`, or says why it could not. The admin checks two passwords at once
    /// and refuses more (429): with the suites running side by side, a refused login tries again, for up to a minute.
    async fn open(http: reqwest::Client, origin: String) -> Result<Admin, String> {
        let password = std::env::var("ADMIN_PASSWORD").map_err(|_| "ADMIN_PASSWORD is not set".to_owned())?;
        let login = async {
            let response = loop {
                let request = http.post(format!("{origin}/api/login")).header("origin", &origin);
                let response =
                    request.json(&json!({"password": password})).send().await.map_err(|e| format!("login: {e}"))?;
                if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
                    break response;
                }
                tokio::time::sleep(Duration::from_millis(100 + rand_u64() % 100)).await;
            };
            if !response.status().is_success() {
                return Err(format!("login refused: {}", response.status()));
            }
            let cookie = session_cookie(&response).ok_or("no session cookie")?;
            let body = response.json::<Value>().await.map_err(|e| format!("the login's json: {e}"))?;
            let csrf = body["csrf"].as_str().ok_or("no csrf token")?.to_owned();
            Ok((cookie, csrf))
        };
        let (cookie, csrf) = tokio::time::timeout(Duration::from_secs(60), login)
            .await
            .map_err(|_| "login: still refused or unanswered after a minute".to_owned())??;
        Ok(Admin { http, origin, csrf, cookie, own: Mutex::default() })
    }

    /// Another login of the same client, from its session cookie and CSRF token.
    pub fn with_login(&self, cookie: String, csrf: String) -> Admin {
        Admin { http: self.http.clone(), origin: self.origin.clone(), csrf, cookie, own: Mutex::default() }
    }

    pub async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, Refused> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.origin))
            .header("origin", &self.origin)
            .header("cookie", &self.cookie)
            .header("x-csrf-token", &self.csrf);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|e| Refused(0, e.to_string()))?;
        let status = response.status().as_u16();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        if (200..300).contains(&status) { Ok(value) } else { Err(Refused(status, value.to_string())) }
    }

    /// Fetches a download by its ticket, as the admin page's own link does.
    pub async fn download(&self, session: &str, ticket: &[u8]) -> reqwest::Response {
        self.http
            .get(format!("{}/api/sessions/{session}/files/{}", self.origin, URL_SAFE_NO_PAD.encode(ticket)))
            .header("cookie", &self.cookie)
            .header("sec-fetch-site", "same-origin")
            .send()
            .await
            .expect("the download request")
    }

    /// Ends every session this login started and has not stopped (other tests' sessions on the same admin go on).
    pub async fn clear(&self) {
        let own = self.own.lock().unwrap().clone();
        for id in own {
            self.stop(&id).await;
        }
    }

    pub async fn create(&self, url: &str) -> String {
        self.create_with(json!({"url": url, "devtools": false})).await
    }

    /// A session from the full request body (an egress, for one).
    pub async fn create_with(&self, body: Value) -> String {
        self.clear().await;
        let session = self.call(reqwest::Method::POST, "/api/sessions", Some(body)).await.expect("a session");
        let id = session["id"].as_str().expect("an id").to_owned();
        self.own.lock().unwrap().push(id.clone());
        id
    }

    /// Waits until the session's page titled `title` has loaded (its window shows the title): a click before that
    /// reaches no listener.
    pub async fn wait_title(&self, session: &str, title: &str) {
        let shown = format!("{title} - ");
        let loaded =
            |windows: &[Value]| windows.iter().any(|w| w["title"].as_str().is_some_and(|t| t.starts_with(&shown)));
        self.wait_windows(session, &format!("the page {title:?}"), |windows| loaded(windows).then_some(())).await
    }

    /// Polls the windows the agent lists, through transient refusals (an agent not yet attached), until `found` picks
    /// something from them; fails after a minute.
    pub async fn wait_windows<T>(&self, session: &str, what: &str, mut found: impl FnMut(&[Value]) -> Option<T>) -> T {
        let listed = until!(
            120,
            Duration::from_millis(500),
            match self.agent(session, "list_windows", json!({})).await {
                Ok(listed) => found(listed["structuredContent"]["windows"].as_array().map_or(&[], Vec::as_slice)),
                Err(_) => None,
            }
        );
        listed.unwrap_or_else(|| panic!("{what} never showed in the agent's window list"))
    }

    /// A new token named `name`, as an agent holds it; it is never listed.
    pub async fn token(&self, name: &str) -> Agent {
        let created =
            self.call(reqwest::Method::POST, "/api/tokens", Some(json!({"name": name}))).await.expect("a token");
        let token = created["token"].as_str().expect("the token, once").to_owned();
        assert!(token.starts_with("easi_"));
        let listed = self.get("/api/tokens").await.unwrap().to_string();
        assert!(!listed.contains(&token[5..]), "the token is never listed");
        Agent {
            http: self.http.clone(),
            origin: self.origin.clone(),
            token,
            id: created["id"].as_str().unwrap().into(),
        }
    }

    /// Deletes every item listed at `/api/{kind}` (`profiles`, `tokens`) whose name `named` picks.
    pub async fn remove_named(&self, kind: &str, named: impl Fn(&str) -> bool) {
        let listed = self.get(&format!("/api/{kind}")).await.unwrap_or_else(|e| panic!("the {kind}: {e:?}"));
        for item in listed[kind].as_array().into_iter().flatten().filter(|i| i["name"].as_str().is_some_and(&named)) {
            let path = format!("/api/{kind}/{}", item["id"].as_str().unwrap());
            self.call(reqwest::Method::DELETE, &path, None).await.unwrap_or_else(|e| panic!("{path}: {e:?}"));
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value, Refused> {
        self.call(reqwest::Method::GET, path, None).await
    }

    /// Counts a session another principal of this admin started (a token's, a local MCP client's) as this login's own,
    /// so that it is stopped with them if the test fails before stopping it.
    pub fn adopt(&self, session: &str) {
        self.own.lock().unwrap().push(session.to_owned());
    }

    /// Stops a session; once it is gone (stopped now, or not there), this login no longer counts it as its own.
    pub async fn stop(&self, session: &str) {
        let stopped = self.call(reqwest::Method::DELETE, &format!("/api/sessions/{session}"), None).await;
        if matches!(stopped, Ok(_) | Err(Refused(404, _))) {
            self.own.lock().unwrap().retain(|id| id != session);
        }
    }

    pub async fn agent(&self, session: &str, tool: &str, arguments: Value) -> Result<Value, Refused> {
        let body = json!({"tool": tool, "arguments": arguments});
        self.call(reqwest::Method::POST, &format!("/api/sessions/{session}/agent"), Some(body)).await
    }

    pub async fn take_control(&self, session: &str, viewer: &Viewer) -> Result<Value, Refused> {
        let body = json!({"viewer": viewer.token});
        self.call(reqwest::Method::POST, &format!("/api/sessions/{session}/control"), Some(body)).await
    }

    pub async fn give_back(&self, session: &str, viewer: &Viewer) -> Result<Value, Refused> {
        let body = json!({"viewer": viewer.token});
        self.call(reqwest::Method::POST, &format!("/api/sessions/{session}/release"), Some(body)).await
    }

    /// Takes control for `viewer` and waits until it is told it holds it at the epoch granted (it may hold it already,
    /// at an older one: a viewer opened on an idle agent's session); returns that lease epoch.
    pub async fn take(&self, session: &str, viewer: &Viewer) -> u32 {
        let granted = self.take_control(session, viewer).await.expect("control");
        let epoch = u32::try_from(granted["lease_epoch"].as_u64().expect("the lease epoch")).expect("an epoch");
        let told = until!(200, Duration::from_millis(50), viewer.lease().filter(|l| *l == (epoch, LeaseHolder::You)));
        told.unwrap_or_else(|| panic!("the viewer was never told it holds epoch {epoch} (last: {:?})", viewer.lease()));
        epoch
    }

    /// Gives control back for `viewer` and waits until it is told the agent holds it.
    pub async fn hand_back(&self, session: &str, viewer: &Viewer) {
        self.give_back(session, viewer).await.expect("give back");
        viewer.wait_lease(LeaseHolder::Agent).await;
    }

    /// A session on `url` and a viewer of it (lossless tiles) that takes control once the page titled `title` has
    /// loaded: the session, the viewer, and its lease epoch. The agent reads the title first, so the viewer opens
    /// watching it, as a person does while the agent acts.
    pub async fn hold(&self, url: &str, title: &str) -> (String, Viewer, u32) {
        self.hold_with(json!({"url": url, "devtools": false}), title).await
    }

    /// [`Admin::hold`] from the full request body.
    pub async fn hold_with(&self, body: Value, title: &str) -> (String, Viewer, u32) {
        let session = self.create_with(body).await;
        self.wait_title(&session, title).await;
        let viewer = self.watch(&session, 0).await;
        viewer.wait_lease(LeaseHolder::Agent).await;
        let held = self.take(&session, &viewer).await;
        (session, viewer, held)
    }

    /// Opens a viewer WebSocket and says hello (lossless tiles only unless `codecs` says otherwise). It acknowledges
    /// every video, tiles and cursor message as it arrives, as the web client does, unless told to withhold them.
    pub async fn watch(&self, session: &str, codecs: u8) -> Viewer {
        let token = format!("{:016x}{:016x}", rand_u64(), rand_u64().rotate_left(17) ^ session.len() as u64);
        let url = format!("{}/ws/sessions/{session}?viewer={token}", self.origin.replacen("https", "wss", 1));
        let mut request = url.into_client_request().expect("a request");
        request.headers_mut().insert("origin", self.origin.parse().expect("origin"));
        request.headers_mut().insert("cookie", self.cookie.parse().expect("cookie"));
        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls()));
        let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .expect("the viewer WebSocket");
        let (mut tx, mut rx) = socket.split();
        let state = Arc::new(Mutex::new(Seen::default()));
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
        let seen = state.clone();
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pause = paused.clone();
        let withheld = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let withhold = withheld.clone();
        tokio::spawn(async move {
            // The newest media revision received, and the newest acknowledged.
            let (mut received, mut acknowledged) = (0u64, 0u64);
            loop {
                while pause.load(std::sync::atomic::Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                if received > acknowledged && !withhold.load(std::sync::atomic::Ordering::Relaxed) {
                    let ack = Message::MediaAck { revision: received }.encode();
                    if tx.send(tungstenite::Message::Binary(ack.into())).await.is_err() {
                        break;
                    }
                    acknowledged = received;
                }
                tokio::select! {
                    frame = rx.next() => match frame {
                        Some(Ok(tungstenite::Message::Binary(bytes))) => {
                            // Decoded while holding the lock: a message that does not decode panics this task and
                            // poisons `seen`, so every later read of what the viewer saw fails instead of finding
                            // nothing (a check that nothing arrived would otherwise pass).
                            let mut seen = seen.lock().unwrap();
                            let message = Message::decode(&bytes, Direction::ServerToClient);
                            let message = message.expect("a valid server message");
                            if let Message::Video { revision, .. }
                            | Message::Tiles { revision, .. }
                            | Message::Cursor { revision, .. } = message
                            {
                                received = revision;
                            }
                            seen.record(message);
                        }
                        Some(Ok(_)) => {}
                        _ => { seen.lock().unwrap().closed = true; break; }
                    },
                    outgoing = out_rx.recv() => match outgoing {
                        Some(bytes) => if tx.send(tungstenite::Message::Binary(bytes.into())).await.is_err() { break },
                        None => { let _ = tx.send(tungstenite::Message::Close(None)).await; break; }
                    },
                    // Acknowledgements withheld go out soon after they no longer are, media or not.
                    _ = tokio::time::sleep(Duration::from_millis(20)), if received > acknowledged => {}
                }
            }
        });
        let viewer = Viewer { token, out: out_tx, seen: state, paused, withheld };
        viewer.send(Message::Hello { version: VERSION, codecs }).await;
        viewer
    }
}

impl Admin {
    /// Opens a viewer as [`Admin::watch`] does and leaves control to the agent: a viewer opened on a session nobody holds
    /// whose agent has not acted lately holds control at once, and this one gives it back, so the agent may act while
    /// it watches (from before the page loads, say).
    pub async fn watch_leaving_control(&self, session: &str, codecs: u8) -> Viewer {
        let viewer = self.watch(session, codecs).await;
        let first = viewer.wait_for("the viewer's first lease", |s| s.leases.first().map(|l| l.1)).await;
        if first == LeaseHolder::You {
            self.hand_back(session, &viewer).await;
        }
        viewer
    }
}

/// A login stops the sessions it started and has not stopped when it goes (a test that failed midway leaves some), so
/// none stays to hold a place of the admin's session limit, which the suites running beside it share.
impl Drop for Admin {
    fn drop(&mut self) {
        let own = std::mem::take(self.own.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner));
        if own.is_empty() {
            return;
        }
        let origin = self.origin.clone();
        // The test's runtime may be unwinding, and drop cannot await on it anyway: stop them from a runtime of their
        // own, without panicking (which would abort a run that is unwinding), and with a login of their own, since this
        // one may have ended (logged out, or by the second step). That login starts nothing, so its own drop is idle.
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            let Ok(http) = client().timeout(Duration::from_secs(60)).build() else {
                return;
            };
            runtime.block_on(async move {
                if let Ok(admin) = Admin::open(http, origin).await {
                    for id in own {
                        admin.stop(&id).await;
                    }
                }
            });
        })
        .join();
    }
}

/// The admin's session cookie (`__Host-display=...`) a response sets, if it sets one.
pub fn session_cookie(response: &reqwest::Response) -> Option<String> {
    let mut cookies = response.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok());
    cookies.find_map(|v| v.split(';').next().filter(|c| c.starts_with("__Host-display="))).map(str::to_owned)
}

/// An agent holding a token: requests carry it and nothing else (no cookie, no CSRF token, no Origin).
#[derive(Clone)]
pub struct Agent {
    pub http: reqwest::Client,
    pub origin: String,
    pub token: String,
    pub id: String,
}

impl Agent {
    /// A request to the agent API with the token: its status and answer.
    pub async fn rest(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (reqwest::StatusCode, Value) {
        let mut request = self.http.request(method, format!("{}{path}", self.origin)).bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.expect("the agent API answers");
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64);
    h.finish()
}

#[derive(Default)]
pub struct Seen {
    pub lease: Option<(u32, LeaseHolder)>,
    /// Every lease the viewer was told, in order.
    pub leases: Vec<(u32, LeaseHolder)>,
    pub configs: u32,
    pub closed: bool,
    /// Text the browser copied to this viewer, with its lease epoch.
    pub copied: Vec<(u32, String)>,
    /// Downloads offered to this viewer: lease epoch, file, name, size; their tickets: lease epoch, file, ticket; and
    /// their ends: file, size (or none: gone).
    pub offers: Vec<(u32, u32, String, Option<u64>)>,
    pub links: Vec<(u32, u32, [u8; display_protocol::FILE_TICKET])>,
    pub ends: Vec<(u32, Option<u64>)>,
    /// Pieces of downloads that reached this WebSocket (none should: they go to the download endpoint).
    pub pieces: u32,
    /// File chooser requests: lease epoch, request.
    pub requests: Vec<(u32, u32)>,
    /// Upload acknowledgements (upload, bytes) and results (upload, stored).
    pub acks: Vec<(u32, u64)>,
    pub uploaded: Vec<(u32, bool)>,
    /// Every media, sound, and configuration message, in arrival order.
    pub log: Vec<(std::time::Instant, Message)>,
    /// The direct path's offers, and the generations its media moved to and fell back from, as the WebSocket said,
    /// each with the length the log had then.
    pub udp_offers: Vec<(u32, [u8; 32], String)>,
    pub moved: Vec<(u32, usize)>,
    pub fell_back: Vec<(u32, usize)>,
    /// Media and sound messages that came over the direct path.
    pub direct: u32,
}

impl Seen {
    pub fn record(&mut self, message: Message) {
        if matches!(
            message,
            Message::Config { .. }
                | Message::Tiles { .. }
                | Message::Video { .. }
                | Message::Cursor { .. }
                | Message::Audio { .. }
        ) {
            self.log.push((std::time::Instant::now(), message.clone()));
        }
        match message {
            Message::Lease { lease_epoch, holder } => {
                self.lease = Some((lease_epoch, holder));
                self.leases.push((lease_epoch, holder));
            }
            Message::Config { .. } => self.configs += 1,
            Message::Copied { lease_epoch, text } => self.copied.push((lease_epoch, text)),
            Message::FileOffer { lease_epoch, file, size, name } => self.offers.push((lease_epoch, file, name, size)),
            Message::FileLink { lease_epoch, file, ticket } => self.links.push((lease_epoch, file, ticket)),
            Message::FileEnd { file, size, .. } => self.ends.push((file, Some(size))),
            Message::FileGone { file, .. } => self.ends.push((file, None)),
            Message::FileData { .. } => self.pieces += 1,
            Message::FileRequest { lease_epoch, request, .. } => self.requests.push((lease_epoch, request)),
            Message::UploadAck { upload, received, .. } => self.acks.push((upload, received)),
            Message::UploadDone { upload, stored, .. } => self.uploaded.push((upload, stored)),
            Message::UdpOffer { generation, cert_sha256, url } => self.udp_offers.push((generation, cert_sha256, url)),
            Message::MediaMoved { generation } => self.moved.push((generation, self.log.len())),
            Message::MediaFallback { generation } => self.fell_back.push((generation, self.log.len())),
            _ => {}
        }
    }
}

pub struct Viewer {
    pub token: String,
    out: mpsc::Sender<Vec<u8>>,
    pub seen: Arc<Mutex<Seen>>,
    /// While set, the viewer stops reading its socket, as a slow client would.
    pub paused: Arc<std::sync::atomic::AtomicBool>,
    /// While set, the viewer reads its socket but acknowledges no media, as a client whose acknowledgements are held up
    /// on the way would; the newest is acknowledged as soon as it is cleared.
    pub withheld: Arc<std::sync::atomic::AtomicBool>,
}

impl Viewer {
    /// Sends bytes as they are, valid protocol or not.
    pub async fn send_raw(&self, bytes: Vec<u8>) {
        self.out.send(bytes).await.expect("the viewer is open");
    }

    pub async fn send(&self, message: Message) {
        self.out.send(message.encode()).await.expect("the viewer is open");
    }

    pub fn lease(&self) -> Option<(u32, LeaseHolder)> {
        self.seen.lock().unwrap().lease
    }

    /// Waits until the viewer has been told `holder`, and returns that lease epoch.
    pub async fn wait_lease(&self, holder: LeaseHolder) -> u32 {
        let told = until!(200, Duration::from_millis(50), self.lease().filter(|l| l.1 == holder).map(|l| l.0));
        told.unwrap_or_else(|| panic!("the viewer was never told {holder:?} (last: {:?})", self.lease()))
    }

    /// Waits until `found` picks something from what the viewer has seen; fails after ten seconds.
    pub async fn wait_for<T>(&self, what: &str, mut found: impl FnMut(&Seen) -> Option<T>) -> T {
        let arrived = until!(200, Duration::from_millis(50), found(&self.seen.lock().unwrap()));
        arrived.unwrap_or_else(|| panic!("{what} never arrived"))
    }

    /// Types text through physical keys of the US layout (letters, digits, `-`).
    pub async fn type_keys(&self, epoch: u32, text: &str) {
        for c in text.chars() {
            let (code, shift) = us_key(c).expect("a typeable character");
            let shifted = [display_protocol::keys::LEFTSHIFT, code];
            self.chord(epoch, if shift { &shifted } else { &shifted[1..] }).await;
        }
    }

    /// Presses `keys` in order and releases them in reverse, at once.
    pub async fn chord(&self, epoch: u32, keys: &[u16]) {
        for &key in keys {
            self.key(epoch, key, true).await;
        }
        for &key in keys.iter().rev() {
            self.key(epoch, key, false).await;
        }
    }

    pub async fn key(&self, epoch: u32, code: u16, down: bool) {
        let kind = if down { display_protocol::KeyKind::Down } else { display_protocol::KeyKind::Up };
        self.send(Message::Key { lease_epoch: epoch, kind, code, text: String::new() }).await;
    }

    pub async fn click(&self, epoch: u32, x: u16, y: u16) {
        use display_protocol::PointerKind;
        for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
            self.pointer(epoch, kind, x, y).await;
        }
    }

    /// The pointer's `kind` of event at (`x`, `y`), with no button.
    pub async fn pointer(&self, epoch: u32, kind: display_protocol::PointerKind, x: u16, y: u16) {
        self.send(Message::Pointer { lease_epoch: epoch, kind, button: 0, x, y }).await;
    }

    /// Every video payload received, in order: an Annex B stream (H.264) or a low-overhead OBU stream (AV1).
    pub fn video(&self) -> Vec<u8> {
        let seen = self.seen.lock().unwrap();
        seen.log
            .iter()
            .filter_map(|(_, m)| if let Message::Video { payload, .. } = m { Some(payload.clone()) } else { None })
            .flatten()
            .collect()
    }

    /// Every sound packet received, in order, with its arrival time and timestamp.
    pub fn audio(&self) -> Vec<(std::time::Instant, u64, Vec<u8>)> {
        let seen = self.seen.lock().unwrap();
        seen.log
            .iter()
            .filter_map(|(t, m)| match m {
                Message::Audio { timestamp_us, payload } => Some((*t, *timestamp_us, payload.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn log(&self) -> Vec<(std::time::Instant, Message)> {
        self.seen.lock().unwrap().log.clone()
    }

    /// Whether lossless tiles arrived after the first `from` messages of the log: the screen was redrawn.
    pub fn redrew(&self, from: usize) -> bool {
        self.seen.lock().unwrap().log.iter().skip(from).any(|(_, m)| matches!(m, Message::Tiles { .. }))
    }

    pub async fn close(self) {
        drop(self.out);
    }
}

fn us_key(c: char) -> Option<(u16, bool)> {
    const ROWS: [(u16, &str); 4] = [(2, "1234567890-"), (16, "qwertyuiop"), (30, "asdfghjkl"), (44, "zxcvbnm")];
    let lower = c.to_ascii_lowercase();
    ROWS.iter().find_map(|(start, row)| row.find(lower).map(|i| (start + i as u16, c.is_ascii_uppercase())))
}

/// Runs the Docker CLI; whether it succeeded, and its standard output followed by its standard error.
pub fn docker(args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new("docker").args(args).output().expect("docker runs");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// The contents of `file` in the cgroup (v2) of the running `container`, found through its main process (where Docker
/// puts it depends on its cgroup driver).
pub fn cgroup_file(container: &str, file: &str) -> String {
    let (ok, pid) = docker(&["inspect", "-f", "{{.State.Pid}}", container]);
    assert!(ok, "docker inspect {container}: {pid}");
    let cgroup = std::fs::read_to_string(format!("/proc/{}/cgroup", pid.trim())).expect("the container's cgroup");
    let path = cgroup.trim().strip_prefix("0::").expect("a cgroup v2 path");
    std::fs::read_to_string(format!("/sys/fs/cgroup{path}/{file}"))
        .unwrap_or_else(|e| panic!("{container}'s {file}: {e}"))
}

/// An empty directory `name` under the test's temporary directory, writable by every uid so a tools container's own
/// can write there too.
pub fn scratch(name: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("an open scratch directory");
    dir
}

/// Runs `args` in the tools image (ffmpeg; bench/tools, which scripts/e2e.sh builds) without a network, with `dir` (a
/// [`scratch`] directory) at `/d`. A missing image fails at once rather than being looked for in a registry.
pub fn tools(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("docker")
        .args(["run", "--rm", "--pull", "never", "--network", "none", "-v"])
        .arg(format!("{}:/d", dir.display()))
        .arg("easi-run/tools:dev")
        .args(args)
        .output()
        .expect("docker runs the tools image")
}

/// The RGB pixels of each rectangle of a `Tiles` message, which must be valid: its records inflate, reading at most one
/// byte past the declared length (so a malformed stream cannot exhaust memory), to exactly that length.
pub fn tiles(rects: &[display_protocol::Rect], raw_len: u32, data: &[u8]) -> Vec<Vec<u8>> {
    use std::io::Read;
    let mut raw = Vec::new();
    let inflate = flate2::read::DeflateDecoder::new(data);
    inflate.take(u64::from(raw_len) + 1).read_to_end(&mut raw).expect("tile records that inflate");
    assert_eq!(raw.len(), raw_len as usize, "tile records of the declared length");
    display_protocol::records::decode(rects, &raw).expect("valid tile records")
}

/// The pid and window id of the first window the agent lists, for [`Admin::wait_windows`].
pub fn first_window(windows: &[Value]) -> Option<(u64, u64)> {
    let w = windows.first()?;
    Some((w["pid"].as_u64()?, w["window_id"].as_u64()?))
}

/// The counter `name` in [`counts`], when it is there.
pub fn count(counters: &str, name: &str) -> Option<u32> {
    counters.split_whitespace().skip_while(|w| *w != name).nth(1).and_then(|n| n.trim_matches('"').parse().ok())
}

/// The typing fixture's input counters (`moves N clicks N ...`), as the agent reads them, or the page's whole text
/// without them.
pub async fn counts(admin: &Admin, session: &str) -> String {
    let text = page_text(admin, session).await;
    text.lines().find(|l| l.contains("moves ")).map_or_else(|| text.clone(), |l| l.trim().to_owned())
}

/// The agent presses Ctrl+`key` in `window`, which it answers without an error, and the page then counts more than
/// `before` keys.
pub async fn agent_ctrl(admin: &Admin, session: &str, (pid, window): (u64, u64), key: &str, before: u32) {
    let keys = json!({"pid": pid, "window_id": window, "keys": ["ctrl", key]});
    let pressed = admin.agent(session, "hotkey", keys).await;
    assert!(pressed.as_ref().is_ok_and(|r| r["isError"] != json!(true)), "the agent's Ctrl+{key}: {pressed:?}");
    // The hotkey returns once the keys are injected; the page counts them a little later.
    let mut counted = String::new();
    let reached = until!(20, Duration::from_millis(500), {
        counted = counts(admin, session).await;
        count(&counted, "keys").is_some_and(|n| n > before).then_some(())
    });
    assert!(reached.is_some(), "the agent's keys did not reach the page: {counted}");
}

/// The text of the session's page, as the agent reads it.
pub async fn page_text(admin: &Admin, session: &str) -> String {
    let text = until!(60, Duration::from_millis(500), {
        let (pid, window) = admin.wait_windows(session, "the page's window", first_window).await;
        let state = admin.agent(session, "get_window_state", json!({"pid": pid, "window_id": window})).await;
        state.ok().and_then(|state| state["structuredContent"]["tree_markdown"].as_str().map(str::to_owned))
    });
    text.unwrap_or_else(|| panic!("the agent never read the page"))
}
