//! Browser sessions, their viewers, and the control lease.
//!
//! The admin application is the lease's authority. A grant gives one viewer connection control at a new epoch and is
//! sent to the display on that connection; every revocation (give back, a disconnect, a ping timeout, a logout, or an
//! expired login) moves to a newer epoch with no holder. Input is forwarded only from the holder at the current
//! epoch, and the display enforces the same rule independently.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use display_protocol::frame::{self, CodecChoice, Link};
use display_protocol::{FILE_TICKET, LeaseHolder, Message};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use crate::agent::Agent;
use crate::auth::{Auth, LoginKey};
use crate::controller::Controller;
use crate::redact::Redactions;

pub struct Config {
    pub origin: String,
    pub state_dir: PathBuf,
    pub static_dir: PathBuf,
    /// Where the web client is served: `/`, or a path such as `/demo` (the page then lives under `/demo/`).
    pub static_path: String,
    /// The viewer SDK (`/sdk/`), served whatever page `static_dir` holds, so a custom client can build on it.
    pub sdk_dir: PathBuf,
    pub streams_dir: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Where every browser says it is: its time zone and languages (the controller checks both).
    pub locale: Locale,
    /// Browser sessions at once, starting ones included.
    pub max_sessions: usize,
    /// The public demo, when this installation is one (see `demo`).
    pub demo: Option<crate::demo::Demo>,
    /// The pages that may show the demo in a frame (`https://` origins), none for any other installation.
    pub frame_ancestors: Vec<String>,
    /// Whether the local MCP socket is served (see `local`): never in the demo.
    pub local_mcp: bool,
    /// Where viewers reach the direct path, `https://host:port`, when it is on (see `udp`).
    pub udp: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    pub timezone: String,
    pub languages: Vec<String>,
}

/// Lock order, outermost first: `sessions`, a session's `state`, `auth`, `tokens`, `profiles`, `egresses`. The demo's visits lock
/// stands alone: it is never held with any of them, either way. Taking one out of this order (even through a guard an `if let` condition
/// keeps alive into its body) can deadlock.
pub struct App {
    pub config: Config,
    pub auth: std::sync::Mutex<Auth>,
    pub controller: Controller,
    pub sessions: std::sync::Mutex<HashMap<String, Arc<Session>>>,
    /// Sessions being created, which count toward `max_sessions` until they are listed or undone.
    pub starting: std::sync::atomic::AtomicUsize,
    pub next_viewer: AtomicU64,
    /// Download responses being served, across every session: each holds up to `FILE_WINDOW` bytes.
    pub downloads: Arc<std::sync::atomic::AtomicUsize>,
    /// The demo's visits and places (unused outside the demo).
    pub demo_visits: crate::demo::Visits,
    /// The agent tokens.
    pub tokens: std::sync::Mutex<crate::tokens::Tokens>,
    /// The saved profiles (see `profiles`).
    pub profiles: std::sync::Mutex<crate::profiles::Profiles>,
    /// The egresses (see `egresses`).
    pub egresses: std::sync::Mutex<crate::egresses::Egresses>,
    /// The direct path, when it is on (see `udp`). Its tickets' lock stands alone, as the demo's visits' does.
    pub udp: Option<crate::udp::Udp>,
}

/// What a session's viewers may do, beyond watching, fixed when the session starts. The relay enforces it: what a
/// feature that is off would carry is dropped in both directions, whatever the client sends. It governs viewers only;
/// the agent's own calls do not pass through here.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Features {
    /// Pointer, wheel and keys from the control holder.
    pub keyboard_pointer: bool,
    /// Text copied on the page to the holder, and pasted from the holder.
    pub clipboard: bool,
    /// Files from the holder: drops and the page's file chooser.
    pub uploads: bool,
    /// Files the page downloads, to the holder.
    pub downloads: bool,
    /// The page's sound, live to viewers and in recordings.
    pub sound: bool,
    /// Recordings by the holder; a recording is a download, so it needs `downloads` too.
    pub recording: bool,
}

impl Default for Features {
    fn default() -> Features {
        Features {
            keyboard_pointer: true,
            clipboard: true,
            uploads: true,
            downloads: true,
            sound: true,
            recording: true,
        }
    }
}

impl Features {
    /// The features as they take effect: recording only with downloads.
    pub fn effective(self) -> Features {
        Features { recording: self.recording && self.downloads, ..self }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub codec: Codec,
    pub bitrate_kbps: u32,
    pub max_fps: u8,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Auto,
    Av1,
    H264,
    TilesOnly,
}

impl Settings {
    pub fn link(&self) -> Link {
        let codec = match self.codec {
            Codec::Auto => CodecChoice::Auto,
            Codec::Av1 => CodecChoice::Av1,
            Codec::H264 => CodecChoice::H264,
            Codec::TilesOnly => CodecChoice::TilesOnly,
        };
        Link::Settings { codec, bitrate_kbps: self.bitrate_kbps, max_fps: self.max_fps }
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { codec: Codec::Auto, bitrate_kbps: frame::DEFAULT_BITRATE_KBPS, max_fps: frame::DEFAULT_MAX_FPS }
    }
}

/// The network conditions simulated on this session's viewer streams.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Network {
    pub latency_ms: u32,
    pub loss_percent: f32,
    pub bandwidth_kbps: u32,
}

/// What a viewer connection receives from outside its relay.
pub enum ToViewer {
    Lease(Message),
    Display(Link),
    Close(&'static str),
    /// Open `url` in place of the browser's pages, for this viewer while it holds the lease at `lease_epoch` (a demo
    /// switch); `opened` hears whether the browser did.
    Open {
        lease_epoch: u32,
        url: String,
        opened: oneshot::Sender<bool>,
    },
    /// The person's browser fetches a download: its relay sends the bytes here instead of to the WebSocket, and
    /// answers whether it took it.
    Download(Diversion, oneshot::Sender<bool>),
    /// The direct path's session this viewer opened with the ticket of offer `generation` (see `udp`).
    Udp {
        generation: u32,
        connection: wtransport::Connection,
    },
}

/// What the relay hands the download response: the pieces in order, then the end, or a failure.
pub enum Piece {
    Data(Bytes),
    End,
    Fail,
}

/// One download taken by the download endpoint.
pub struct Diversion {
    pub file: u32,
    pub lease_epoch: u32,
    /// The size it was offered with, which it must have.
    pub size: Option<u64>,
    pub pieces: mpsc::Sender<Piece>,
    /// Bytes the response took so far, and whether it was dropped; the relay is woken on either.
    pub consumed: Arc<AtomicU64>,
    pub dropped: Arc<AtomicBool>,
    /// The response took the confirmed end: every byte went to the person's browser.
    pub delivered: Arc<AtomicBool>,
}

/// A single-use ticket for one download, given to the holder that was offered it.
pub struct Ticket {
    pub viewer: u64,
    pub login: LoginKey,
    pub lease_epoch: u32,
    pub file: u32,
    pub size: Option<u64>,
    pub name: String,
}

pub struct Viewer {
    pub login: LoginKey,
    /// The client's own name for this connection, used by its control requests.
    pub token: String,
    pub tx: mpsc::Sender<ToViewer>,
    /// Wakes the relay when one of its downloads' responses moved on or went away.
    pub wake: Arc<Notify>,
    pub stats: ViewerStats,
}

#[derive(Default, Clone, Debug, Serialize)]
pub struct ViewerStats {
    pub frames_per_s: u32,
    pub kbit_per_s: u32,
    pub encode_us_p50: u32,
    pub encode_us_p95: u32,
    pub presented_per_s: u32,
    pub decode_us_p50: u32,
    pub hardware: bool,
    pub codec: String,
}

pub struct State {
    pub viewers: HashMap<u64, Viewer>,
    pub lease_epoch: u32,
    pub holder: Option<u64>,
    /// The agent was killed by the last takeover and must be restarted on give back.
    pub agent_terminated: bool,
    pub settings: Settings,
    pub network: Network,
    pub redactions: Redactions,
    pub failed: Option<String>,
    pub tickets: HashMap<[u8; FILE_TICKET], Ticket>,
    /// Since when no viewer has been connected (from the start, until the first one comes); none while one is.
    pub unwatched_since: Option<Instant>,
}

#[cfg(test)]
impl Viewer {
    /// A viewer for a test, of login `a`, and what it is sent.
    pub fn test() -> (Viewer, mpsc::Receiver<ToViewer>) {
        let (tx, rx) = mpsc::channel(16);
        let login = crate::auth::key_of("a");
        (Viewer { login, token: "t".into(), tx, wake: Arc::default(), stats: ViewerStats::default() }, rx)
    }
}

impl State {
    /// Whether `viewer` holds control at `epoch`: the one rule every lease-bound message, ticket, and download obeys.
    pub fn holds(&self, viewer: u64, epoch: u32) -> bool {
        self.holder == Some(viewer) && self.lease_epoch == epoch
    }
}

/// Who a session belongs to, and who asks for one: each session is started by one of them and the server records which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The administrator, by their login: outside the demo.
    Admin,
    /// An agent token, by its id (see `tokens`).
    Token(u64),
    /// Every client of the local MCP socket (`admin mcp`, see `local`), together: one principal with exactly a token's
    /// rights, shared by everyone who can run a command in the admin's container.
    Local,
    /// A demo visitor, by their login.
    Visitor(LoginKey),
}

pub struct Session {
    pub id: String,
    pub url: String,
    pub devtools: bool,
    /// The egress the session leaves through, by its name when the session started; none is direct.
    pub egress: Option<String>,
    /// That egress's id, which the session holds until it is stopped (see `egresses`).
    pub egress_id: Option<String>,
    /// Who started it (see [`App::reachable`]).
    pub owner: Owner,
    /// What its viewers may do (as it takes effect, see [`Features::effective`]).
    pub features: Features,
    /// The saved profile it started from, if any, which it holds until it is stopped.
    pub profile: Option<crate::profiles::ProfileUse>,
    pub agent: Agent,
    pub state: std::sync::Mutex<State>,
    /// Serializes takeovers and give-backs.
    pub control: Mutex<()>,
    /// Viewer connections taken or being upgraded (the demo bounds them per session).
    pub places: Arc<std::sync::atomic::AtomicUsize>,
    /// Set once the session is stopped: every relay of it ends, even one blocked writing to a viewer that stopped
    /// reading.
    pub ended: tokio::sync::watch::Sender<bool>,
    /// The controller's last inspection of its containers, and when it answered (see [`Session::containers`]).
    inspection: Arc<Mutex<Option<(Instant, Value)>>>,
}

/// The first and the longest wait between attempts to remove what earlier runs left.
const CLEANUP_RETRY_FIRST: Duration = Duration::from_millis(500);
const CLEANUP_RETRY_MAX: Duration = Duration::from_secs(5);

/// How long one inspection of a session's containers answers every stats request.
const INSPECTION_FRESH: Duration = Duration::from_secs(2);

/// Why a viewer's stream is closed when its login ended.
pub const ENDED: &str = "the login ended";

/// Why a viewer's stream is closed when its demo visit was closed for one opened in another tab or browser (the
/// client tells it apart by its WebSocket close code, [`MOVED_CODE`]).
pub const MOVED: &str = "the demo opened in another tab";
pub const MOVED_CODE: u16 = 4001;

impl Session {
    pub fn new(
        id: String,
        url: String,
        devtools: bool,
        egress: Option<String>,
        owner: Owner,
        features: Features,
        controller: Controller,
    ) -> Session {
        Session {
            agent: Agent::new(id.clone(), controller),
            id,
            url,
            devtools,
            egress,
            egress_id: None,
            owner,
            features: features.effective(),
            profile: None,
            state: std::sync::Mutex::new(State {
                viewers: HashMap::new(),
                lease_epoch: 1,
                holder: None,
                agent_terminated: false,
                settings: Settings::default(),
                network: Network::default(),
                redactions: Redactions::default(),
                failed: None,
                tickets: HashMap::new(),
                unwatched_since: Some(Instant::now()),
            }),
            control: Mutex::new(()),
            places: Arc::default(),
            ended: tokio::sync::watch::Sender::new(false),
            inspection: Arc::new(Mutex::new(None)),
        }
    }

    /// A session for a test, of `owner` and with `features`, at https://example.com/, whose controller answers nothing.
    #[cfg(test)]
    pub fn test(id: &str, owner: Owner, features: Features) -> Session {
        let controller = Controller::new("/nonexistent".into());
        Session::new(id.into(), "https://example.com/".into(), false, None, owner, features, controller)
    }

    /// Its containers as the controller inspected them at most [`INSPECTION_FRESH`] ago. One inspection runs at a time
    /// however many viewers poll, so polling never fills the controller's connections. The one that refreshes runs as
    /// its own task, holding the cache: a poll that disconnects never cancels it while the controller still works on it
    /// (the next poll would start another), and polls only wait, without a task each.
    pub async fn containers(&self, controller: &Controller) -> Value {
        let mut cached = self.inspection.clone().lock_owned().await;
        if let Some((_, value)) = cached.as_ref().filter(|(at, _)| at.elapsed() < INSPECTION_FRESH) {
            return value.clone();
        }
        let (id, controller) = (self.id.clone(), controller.clone());
        let refreshed = tokio::spawn(async move {
            let value = controller.inspect_session(&id).await.unwrap_or_else(|e| json!({"error": e}));
            *cached = Some((Instant::now(), value.clone()));
            value
        });
        refreshed.await.unwrap_or_else(|_| json!({"error": "the inspection failed"}))
    }

    /// Tells every viewer who holds control now.
    pub fn announce(state: &State) {
        for (id, viewer) in &state.viewers {
            let holder = match state.holder {
                None => LeaseHolder::Agent,
                Some(h) if h == *id => LeaseHolder::You,
                Some(_) => LeaseHolder::Other,
            };
            let _ = viewer.tx.try_send(ToViewer::Lease(Message::Lease { lease_epoch: state.lease_epoch, holder }));
        }
    }

    /// Grants the lease to `viewer`, which must be one of `state.viewers`, at a new epoch, and returns it. Whatever the
    /// previous holder typed is a finished run, as at every other lease change; from now on screenshots never leave.
    pub fn grant(state: &mut State, viewer: u64) -> u32 {
        state.redactions.boundary();
        state.lease_epoch += 2;
        state.holder = Some(viewer);
        state.redactions.screenshots_blocked = true;
        let grant = Link::Grant { lease_epoch: state.lease_epoch, holder: true };
        let _ = state.viewers[&viewer].tx.try_send(ToViewer::Display(grant));
        Session::announce(state);
        state.lease_epoch
    }

    /// Revokes the lease (if any) at a new epoch, through any viewer's display link.
    pub fn revoke(state: &mut State) {
        if state.holder.is_none() {
            return;
        }
        state.holder = None;
        // The display moves its own epoch on when the holder's link closes; stay ahead of it.
        state.lease_epoch += 2;
        let grant = Link::Grant { lease_epoch: state.lease_epoch, holder: false };
        if let Some(viewer) = state.viewers.values().next() {
            let _ = viewer.tx.try_send(ToViewer::Display(grant));
        }
        Session::announce(state);
    }

    /// Revokes the lease because its holder went away (a disconnect, a ping timeout, a logout, an expired login). The
    /// agent may act again, restarted if a takeover killed it.
    pub fn revoke_lost_holder(self: &Arc<Session>, state: &mut State) {
        if state.holder.is_none() {
            return;
        }
        state.redactions.boundary();
        Session::revoke(state);
        let session = self.clone();
        tokio::spawn(async move {
            let _serial = session.control.lock().await;
            // A new takeover may have happened meanwhile.
            session.resume_unheld().await;
        });
    }

    /// Lets the agent act again, restarted if a takeover killed it, unless a viewer holds control (then the agent stays
    /// blocked, and whether it must be restarted stays for that holder's give-back). Called under `control`.
    pub async fn resume_unheld(&self) {
        let terminated = {
            let mut state = self.state.lock().unwrap();
            if state.holder.is_some() {
                return;
            }
            std::mem::take(&mut state.agent_terminated)
        };
        if let Err(why) = self.agent.resume(terminated).await {
            self.state.lock().unwrap().failed = Some(why);
        }
    }

    pub fn viewer_by_token(state: &State, login: LoginKey, token: &str) -> Option<u64> {
        state.viewers.iter().find(|(_, v)| v.login == login && v.token == token).map(|(id, _)| *id)
    }
}

impl App {
    /// The admin's state, with no session yet.
    pub fn new(
        config: Config,
        auth: Auth,
        controller: Controller,
        tokens: crate::tokens::Tokens,
        profiles: crate::profiles::Profiles,
        egresses: crate::egresses::Egresses,
    ) -> Arc<App> {
        let udp = config.udp.clone().map(crate::udp::Udp::new);
        Arc::new(App {
            udp,
            config,
            auth: std::sync::Mutex::new(auth),
            controller,
            sessions: std::sync::Mutex::default(),
            starting: std::sync::atomic::AtomicUsize::new(0),
            next_viewer: AtomicU64::new(0),
            downloads: Arc::default(),
            demo_visits: crate::demo::Visits::default(),
            tokens: std::sync::Mutex::new(tokens),
            profiles: std::sync::Mutex::new(profiles),
            egresses: std::sync::Mutex::new(egresses),
        })
    }

    pub fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    /// Who a login is: in the demo a visitor, anywhere else the administrator.
    pub fn principal(&self, login: LoginKey) -> Owner {
        if self.config.demo.is_some() { Owner::Visitor(login) } else { Owner::Admin }
    }

    /// Whether `who` may still act: a token only while it is live. The local socket's clients always may: the socket is
    /// served only when `ADMIN_LOCAL_MCP` is on, which stays as it was for as long as the admin runs.
    pub fn may_act(&self, who: Owner) -> bool {
        match who {
            Owner::Token(id) => self.tokens.lock().unwrap().is_live(id),
            Owner::Admin | Owner::Local | Owner::Visitor(_) => true,
        }
    }

    /// Whether `who` may reach a session: the administrator every one, an agent token, the local socket's clients or a
    /// demo visitor only its own.
    pub fn reachable(&self, who: Owner, session: &Session) -> bool {
        who == Owner::Admin || who == session.owner
    }

    /// The relay of the viewer a direct path's ticket was issued to, while it is still connected and its login live.
    pub fn udp_viewer(&self, ticket: &crate::udp::Ticket) -> Option<mpsc::Sender<ToViewer>> {
        let session = self.session(&ticket.session)?;
        let state = session.state.lock().unwrap();
        let viewer = state.viewers.get(&ticket.viewer)?;
        self.auth.lock().unwrap().is_live(&viewer.login).then(|| viewer.tx.clone())
    }

    /// A session as `who` may reach it (see [`App::reachable`]).
    pub fn session_for(&self, who: Owner, id: &str) -> Option<Arc<Session>> {
        self.session(id).filter(|s| self.reachable(who, s))
    }

    /// Removes every session container, volume, and network an earlier run of this installation left, then their
    /// stream directories. It retries until the controller confirms the removal: at startup its socket may not be up
    /// yet, and a session left running would be listed nowhere, out of reach of every stop.
    pub async fn clean_up_earlier_runs(&self) {
        let mut wait = CLEANUP_RETRY_FIRST;
        loop {
            match self.controller.cleanup().await {
                Ok(report) => {
                    tracing::info!(%report, "removed what earlier runs left behind");
                    break;
                }
                Err(why) => {
                    tracing::warn!(%why, retry_in = ?wait, "removing what earlier runs left failed; retrying");
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(CLEANUP_RETRY_MAX);
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&self.config.streams_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Closes every stream of a login that ended, saying `why` ([`ENDED`] but for a demo visit), revoking any lease it
    /// held.
    pub fn end_login(&self, login: LoginKey, why: &'static str) {
        for session in self.sessions.lock().unwrap().values() {
            let mut state = session.state.lock().unwrap();
            let held = state.holder.is_some_and(|h| state.viewers.get(&h).is_some_and(|v| v.login == login));
            if held {
                session.revoke_lost_holder(&mut state);
            }
            for viewer in state.viewers.values().filter(|v| v.login == login) {
                let _ = viewer.tx.try_send(ToViewer::Close(why));
            }
        }
    }
}

/// One of at most `max` places that `counter` counts, given back when dropped. `C` is how the place holds the counter:
/// borrowed, or shared to outlive the request that took it.
pub struct Place<C: std::ops::Deref<Target = std::sync::atomic::AtomicUsize>>(C);

impl<C: std::ops::Deref<Target = std::sync::atomic::AtomicUsize>> Place<C> {
    pub fn take(counter: C, max: usize) -> Option<Place<C>> {
        counter.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < max).then_some(n + 1)).ok()?;
        // Built only once taken: a refused one must give nothing back.
        Some(Place(counter))
    }
}

impl<C: std::ops::Deref<Target = std::sync::atomic::AtomicUsize>> Drop for Place<C> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A place for one session being created, taken under the sessions lock so concurrent creates cannot exceed
/// `max_sessions`; it is given back when dropped.
pub fn reserve(app: &App) -> Option<Place<&std::sync::atomic::AtomicUsize>> {
    let sessions = app.sessions.lock().unwrap();
    Place::take(&app.starting, app.config.max_sessions.saturating_sub(sessions.len()))
}

/// An application with no controller behind it, for tests: the demo's (one site) or an ordinary installation's.
#[cfg(test)]
pub fn test_app(demo: bool) -> TestApp {
    test_app_with(demo, Controller::new("/nonexistent".into()))
}

/// A [`test_app`] whose controller is `controller` (a test's fake one).
#[cfg(test)]
pub fn test_app_with(demo: bool, controller: Controller) -> TestApp {
    let state_dir = crate::private::test_dir("app");
    let demo =
        demo.then(|| crate::demo::Demo { seconds: 60, sites: vec![("one".into(), "https://one.bench/".into())] });
    let config = Config {
        origin: "https://localhost:9443".into(),
        static_dir: state_dir.join("static"),
        static_path: "/".into(),
        sdk_dir: state_dir.join("sdk"),
        streams_dir: state_dir.join("streams"),
        width: 1280,
        height: 800,
        locale: Locale { timezone: "UTC".into(), languages: vec!["en-US".into()] },
        max_sessions: 4,
        local_mcp: demo.is_none(),
        udp: None,
        demo,
        frame_ancestors: Vec::new(),
        state_dir: state_dir.to_path_buf(),
    };
    let mut auth = Auth::load(&state_dir, config.demo.is_none()).unwrap();
    auth.offer_passkeys(&config.origin);
    let tokens = crate::tokens::Tokens::load(&state_dir).unwrap();
    let profiles = crate::profiles::Profiles::load(&state_dir).unwrap();
    let egresses = crate::egresses::Egresses::load(&state_dir).unwrap();
    let app = App::new(config, auth, controller, tokens, profiles, egresses);
    TestApp { app, _state: state_dir }
}

/// A [`test_app`], with the state directory it removes once dropped.
#[cfg(test)]
pub struct TestApp {
    app: Arc<App>,
    _state: crate::private::TestDir,
}

#[cfg(test)]
impl std::ops::Deref for TestApp {
    type Target = Arc<App>;
    fn deref(&self) -> &Arc<App> {
        &self.app
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_administrator_reaches_every_session_and_a_token_or_a_visitor_only_its_own() {
        let app = test_app(false);
        let session = |owner| Session::test("abc", owner, Features::default());
        let login = crate::auth::key_of("visitor");
        let (mine, other, visit) = (session(Owner::Token(1)), session(Owner::Admin), session(Owner::Visitor(login)));
        let local = session(Owner::Local);
        for s in [&mine, &other, &visit, &local] {
            assert!(app.reachable(Owner::Admin, s), "the administrator takes over, watches and stops any");
        }
        assert!(app.reachable(Owner::Token(1), &mine));
        assert!(!app.reachable(Owner::Token(2), &mine), "another token never reaches it");
        assert!(!app.reachable(Owner::Token(1), &other) && !app.reachable(Owner::Token(1), &visit));
        assert!(!app.reachable(Owner::Token(1), &local), "a token never reaches a local client's session");
        assert!(app.reachable(Owner::Local, &local), "the local clients reach their own");
        for s in [&mine, &other, &visit] {
            assert!(!app.reachable(Owner::Local, s), "and nothing else");
        }
        assert!(app.reachable(Owner::Visitor(login), &visit) && !app.reachable(Owner::Visitor(login), &other));
        assert_eq!(app.principal(login), Owner::Admin);
        assert_eq!(test_app(true).principal(login), Owner::Visitor(login), "in the demo a login is a visitor");
    }

    #[test]
    fn a_token_may_act_only_while_it_is_live() {
        let app = test_app(false);
        let (listed, token) = app.tokens.lock().unwrap().create("agent").unwrap();
        let id = crate::tokens::parse_id(listed["id"].as_str().unwrap()).unwrap();
        assert!(app.may_act(Owner::Token(id)) && app.may_act(Owner::Admin) && app.may_act(Owner::Local));
        app.tokens.lock().unwrap().revoke(id).unwrap().unwrap();
        assert!(!app.may_act(Owner::Token(id)));
        assert_eq!(app.tokens.lock().unwrap().authenticate(&token), None);
    }

    fn session() -> (Session, Controller) {
        (Session::test("abc", Owner::Admin, Features::default()), Controller::new("/nonexistent".into()))
    }

    #[tokio::test]
    async fn a_fresh_inspection_answers_without_asking_the_controller_again() {
        let (session, controller) = session();
        let first = session.containers(&controller).await;
        assert!(first.get("error").is_some(), "no controller here");
        *session.inspection.lock().await = Some((Instant::now(), json!({"cached": true})));
        assert_eq!(session.containers(&controller).await, json!({"cached": true}));
        *session.inspection.lock().await = Some((Instant::now() - INSPECTION_FRESH, json!({"cached": true})));
        assert!(session.containers(&controller).await.get("error").is_some(), "a stale one is asked for again");
    }

    #[test]
    fn a_takeover_from_another_viewer_ends_the_previous_holder_s_typed_run() {
        let session = session().0;
        let mut state = session.state.lock().unwrap();
        let (viewer, _rx) = Viewer::test();
        state.viewers.insert(2, viewer);
        state.holder = Some(1);
        state.redactions.typed("hunter2-wifi");
        let epoch = Session::grant(&mut state, 2);
        assert_eq!((state.holder, state.lease_epoch, epoch), (Some(2), 3, 3));
        state.redactions.typed("note");
        state.redactions.boundary();
        let page = state.redactions.applied(json!("hunter2-wifi"));
        assert_eq!(page, "[redacted]");
    }

    #[tokio::test]
    async fn a_takeover_before_the_lost_holder_s_resume_keeps_the_agent_s_restart_for_its_give_back() {
        let session = Arc::new(session().0);
        let serial = session.control.lock().await;
        {
            let mut state = session.state.lock().unwrap();
            state.holder = Some(1);
            state.agent_terminated = true;
            session.revoke_lost_holder(&mut state);
        }
        // The resume waits for the control lock, which a new takeover holds and leaves with a new holder.
        tokio::task::yield_now().await;
        session.state.lock().unwrap().holder = Some(2);
        drop(serial);
        let _after = session.control.lock().await;
        assert!(session.state.lock().unwrap().agent_terminated, "its give-back must still restart the agent");
    }

    #[tokio::test]
    async fn the_agent_resumes_only_while_no_viewer_holds_control() {
        let session = session().0;
        let _serial = session.control.lock().await;
        {
            let mut state = session.state.lock().unwrap();
            state.holder = Some(2);
            state.agent_terminated = true;
        }
        session.resume_unheld().await;
        {
            let state = session.state.lock().unwrap();
            assert!(state.agent_terminated && state.failed.is_none(), "the holder's give-back restarts the agent");
        }
        session.state.lock().unwrap().holder = None;
        session.resume_unheld().await;
        let state = session.state.lock().unwrap();
        // No controller here: the restart is attempted, and its failure stops the session.
        assert!(!state.agent_terminated && state.failed.is_some(), "the agent is restarted once nobody holds control");
    }
}
