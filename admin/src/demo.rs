//! The public demo: anyone gets a few minutes with a real browser, without a password, one visit per client at a time.
//!
//! `ADMIN_DEMO_SECONDS` turns it on, with `ADMIN_DEMO_SITES` (`name=https://start/` pairs, comma-separated): the
//! demos a visitor may pick, each a start page in the walled garden the controller enforces; the first is where a
//! visit starts. Visits run side by side, up to `ADMIN_MAX_SESSIONS`. A client is an IPv4 address, or an IPv6 /64
//! network, as Cloudflare's tunnel reports it (the demo admin has no other way in); each holds at most one place: a
//! visit starting, under way, or being cleaned up after. A login first reserves the place; the admin then starts that
//! visitor's only session itself, an ordinary Chrome window, and owns it for the visitor. At the deadline, or when the
//! visitor logs out, the login ends and the session is stopped; the place is given back only once the stop is
//! confirmed. Everything an administrator does (more sessions, settings, the network simulator, agent calls, other
//! visitors' sessions) is refused.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::{App, Session};
use crate::auth::LoginKey;

/// How long between attempts to stop a session whose stop failed; its place stays taken meanwhile.
const STOP_RETRY: Duration = Duration::from_secs(5);
/// Visits one client may start within [`START_WINDOW`].
const MAX_STARTS: usize = 10;
const START_WINDOW: Duration = Duration::from_secs(600);
/// The longest wait a refused visitor is told before asking again.
const MAX_RETRY: u64 = 30;
/// Viewer connections one visitor may hold at once.
pub const MAX_VIEWERS: usize = 2;
const MAX_SITES: usize = 16;

pub struct Demo {
    pub seconds: u64,
    /// `(name, start URL)`, in the order given.
    pub sites: Vec<(String, String)>,
}

impl Demo {
    /// The demo from the environment, or `None` when `ADMIN_DEMO_SECONDS` is unset. Both variables must be valid.
    pub fn from_env(seconds: Option<String>, sites: Option<String>) -> Result<Option<Demo>, String> {
        let (seconds, sites) = match (seconds, sites) {
            (None, None) => return Ok(None),
            (Some(s), Some(l)) => (s, l),
            _ => return Err("ADMIN_DEMO_SECONDS and ADMIN_DEMO_SITES go together".into()),
        };
        let seconds: u64 =
            seconds.parse().ok().filter(|s| (10..=600).contains(s)).ok_or("ADMIN_DEMO_SECONDS must be 10-600")?;
        let mut parsed = Vec::new();
        for pair in sites.split(',') {
            let (name, url) = pair.trim().split_once('=').ok_or("ADMIN_DEMO_SITES: expected name=https://host/")?;
            let name_ok = !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase());
            let url_ok = url.starts_with("https://") && url.len() <= 256 && !url.contains(char::is_whitespace);
            if !name_ok || !url_ok || parsed.iter().any(|(n, _): &(String, String)| n == name) {
                return Err(format!("ADMIN_DEMO_SITES: bad or repeated entry {pair:?}"));
            }
            parsed.push((name.to_owned(), url.to_owned()));
        }
        if parsed.is_empty() || parsed.len() > MAX_SITES {
            return Err(format!("ADMIN_DEMO_SITES must name 1-{MAX_SITES} sites"));
        }
        Ok(Some(Demo { seconds, sites: parsed }))
    }

    pub fn start_url(&self, name: &str) -> Option<&str> {
        self.sites.iter().find(|(n, _)| n == name).map(|(_, url)| url.as_str())
    }
}

/// A demo client: an IPv4 address, or the /64 network of an IPv6 one (a household's share, as one IPv4 address is).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Client {
    V4(Ipv4Addr),
    V6(u64),
}

impl Client {
    pub fn of(ip: IpAddr) -> Client {
        match ip.to_canonical() {
            IpAddr::V4(v4) => Client::V4(v4),
            IpAddr::V6(v6) => Client::V6((u128::from(v6) >> 64) as u64),
        }
    }
}

/// Why a visit cannot start now, with the seconds to wait before asking again.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The demo is still removing what an earlier run left.
    Opening(u64),
    /// This client already holds its place.
    Yours(u64),
    /// Every place is taken.
    Full(u64),
    /// This client started too many visits lately.
    TooOften(u64),
}

/// What a client's place holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    Starting,
    Visiting(LoginKey),
    /// Its session is being stopped; the place is free once that is confirmed.
    Leaving,
}

struct Visit {
    client: Client,
    session: String,
    site: String,
    deadline: Instant,
    /// Held by a switch while its page opens, and by the end of the visit, so switches go one at a time and the
    /// visit's session is stopped only once a switch under way has finished.
    life: Arc<tokio::sync::Mutex<()>>,
    /// Set once the visit's session is confirmed gone, for whoever waits on it (a takeover).
    gone: tokio::sync::watch::Sender<bool>,
    /// Switches asked for (each numbered by this), waiting or under way, and the newest whose page opened: an older
    /// switch's late success never relabels the visit.
    switches: u64,
    switching: usize,
    shown: u64,
}

/// Every visit and every place, for one demo.
#[derive(Default)]
pub struct Visits(std::sync::Mutex<Inner>);

#[derive(Default)]
struct Inner {
    ready: bool,
    visits: HashMap<LoginKey, Visit>,
    places: HashMap<Client, Hold>,
    starts: HashMap<Client, VecDeque<Instant>>,
}

impl Inner {
    /// Seconds until the soonest visit ends, bounded, for a visitor told to wait.
    fn soonest(&self, now: Instant) -> u64 {
        let soonest = self.visits.values().map(|v| v.deadline.saturating_duration_since(now).as_secs() + 1).min();
        soonest.unwrap_or(5).clamp(3, MAX_RETRY)
    }

    /// Counts a visit `client` starts now, unless it started [`MAX_STARTS`] within [`START_WINDOW`] already.
    fn count_start(&mut self, client: Client, now: Instant) -> Result<(), Refused> {
        if self.starts.len() > 10_000 {
            self.starts.retain(|_, s| s.back().is_some_and(|t| now.duration_since(*t) < START_WINDOW));
        }
        let starts = self.starts.entry(client).or_default();
        while starts.front().is_some_and(|t| now.duration_since(*t) >= START_WINDOW) {
            starts.pop_front();
        }
        if starts.len() >= MAX_STARTS {
            let oldest = starts.front().copied().unwrap_or(now);
            return Err(Refused::TooOften(START_WINDOW.saturating_sub(now.duration_since(oldest)).as_secs() + 1));
        }
        starts.push_back(now);
        Ok(())
    }
}

impl Visits {
    /// Takes `client`'s place for a visit starting, if it has none, a place is free among `capacity`, and it has not
    /// started too many visits lately.
    pub fn reserve(&self, client: Client, capacity: usize) -> Result<(), Refused> {
        let mut inner = self.0.lock().unwrap();
        let now = Instant::now();
        if !inner.ready {
            return Err(Refused::Opening(5));
        }
        match inner.places.get(&client) {
            Some(Hold::Visiting(login)) => {
                let left =
                    inner.visits.get(login).map_or(5, |v| v.deadline.saturating_duration_since(now).as_secs() + 1);
                // Asked again soon: the other browser may leave before its time is up.
                return Err(Refused::Yours(left.clamp(3, 10)));
            }
            Some(_) => return Err(Refused::Yours(5)),
            None => {}
        }
        if inner.places.len() >= capacity {
            return Err(Refused::Full(inner.soonest(now)));
        }
        inner.count_start(client, now)?;
        inner.places.insert(client, Hold::Starting);
        Ok(())
    }

    /// For a client opening the demo while its visit is open elsewhere: within its start limit, its place is
    /// kept for the new visit (no other client can take it meanwhile) and the visit to close is returned. None when it
    /// has no visit under way (it then reserves a place as anyone does).
    pub fn take_over(&self, client: Client) -> Result<Option<(LoginKey, Gone)>, Refused> {
        let mut inner = self.0.lock().unwrap();
        let Some(Hold::Visiting(old)) = inner.places.get(&client).copied() else { return Ok(None) };
        let Some(gone) = inner.visits.get(&old).map(|v| v.gone.subscribe()) else { return Ok(None) };
        inner.count_start(client, Instant::now())?;
        inner.places.insert(client, Hold::Starting);
        Ok(Some((old, gone)))
    }

    fn release(&self, client: Client, from: Hold) {
        let mut inner = self.0.lock().unwrap();
        if inner.places.get(&client) == Some(&from) {
            inner.places.remove(&client);
        }
    }
}

/// Opens the demo once whatever an earlier run of this installation left is confirmed gone.
pub fn open(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        app.clean_up_earlier_runs().await;
        app.demo_visits.0.lock().unwrap().ready = true;
    });
}

/// Stops session `id` until the controller confirms it is gone (a session it no longer knows is reclaimed by its
/// labels), however long that takes; the controller's age limit removes it meanwhile if this keeps failing.
pub async fn ensure_stopped(app: &App, id: &str) {
    loop {
        let stopped = match app.session(id) {
            Some(found) => crate::service::stop(app, &found).await,
            None => app.controller.stop_session(id).await,
        };
        match stopped {
            Ok(()) => return,
            Err(why) => {
                tracing::warn!(session = %id, %why, "demo: stopping a session failed; retrying");
                tokio::time::sleep(STOP_RETRY).await;
            }
        }
    }
}

/// Gives back a place whose visit did not start, once session `id` (if it was ever asked for) is confirmed gone.
pub fn failed_start(app: &Arc<App>, client: Client, id: String) {
    let app = app.clone();
    app.demo_visits.0.lock().unwrap().places.insert(client, Hold::Leaving);
    tokio::spawn(async move {
        ensure_stopped(&app, &id).await;
        app.demo_visits.release(client, Hold::Leaving);
    });
}

/// Hands the place to the visitor whose session started (its start having begun at `began`), and arms the deadline.
pub fn activate(
    app: &Arc<App>,
    client: Client,
    login: LoginKey,
    session: String,
    site: String,
    began: Instant,
    seconds: u64,
) {
    let deadline = began + Duration::from_secs(seconds);
    // The login itself dies at the deadline too: every request and the expiry sweep refuse it from then on, even if
    // the timer below were late.
    app.auth.lock().unwrap().set_deadline(login, deadline);
    {
        let mut inner = app.demo_visits.0.lock().unwrap();
        let life = Arc::new(tokio::sync::Mutex::new(()));
        let gone = tokio::sync::watch::Sender::new(false);
        let visit = Visit { client, session, site, deadline, life, gone, switches: 0, switching: 0, shown: 0 };
        inner.visits.insert(login, visit);
        inner.places.insert(client, Hold::Visiting(login));
    }
    let app = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep_until(deadline.into()).await;
        finish(&app, login).await;
    });
}

/// Closes the visit of `old`, taken over by its client opening the demo in another tab or browser (see
/// `Visits::take_over`, which kept the place for the new visit): its streams close with [`crate::app::MOVED`], and
/// this returns once its session is confirmed gone. The newest browser of a client always gets the demo, in a browser
/// of its own; the start limit still bounds how often.
pub async fn close_taken_over(app: &Arc<App>, old: LoginKey, mut gone: Gone) {
    end(app, old, crate::app::MOVED).await;
    // Whoever ended it (this, or its deadline or logout meanwhile), its session is gone before the new one starts.
    let _ = gone.wait_for(|done| *done).await;
}

/// Says when a visit's session is confirmed gone.
pub type Gone = tokio::sync::watch::Receiver<bool>;

/// Ends the visit (at the deadline or on logout): the login ends at once, the session is stopped, and the place is
/// given back only after the stop is confirmed. A second call for the same visitor does nothing.
pub async fn finish(app: &Arc<App>, login: LoginKey) {
    end(app, login, "the login ended").await;
}

/// How long a visit's browser may go without anyone watching before the visit ends: a page that framed the demo and
/// was closed (or a tab that was), so the place goes to the next visitor instead of waiting out the visit. It outlasts
/// a client's reconnecting and a page's reload.
const UNWATCHED_FOR: Duration = Duration::from_secs(30);

/// Ends every visit whose browser nobody has watched for [`UNWATCHED_FOR`].
pub fn end_unwatched(app: &Arc<App>) {
    // The visits are copied out first: the visits lock is never held with another (see [`App`]), a session's state
    // included.
    let visits: Vec<(LoginKey, String)> =
        app.demo_visits.0.lock().unwrap().visits.iter().map(|(login, v)| (*login, v.session.clone())).collect();
    let unwatched = visits.into_iter().filter(|(_, id)| {
        app.session(id)
            .is_some_and(|s| s.state.lock().unwrap().unwatched_since.is_some_and(|t| t.elapsed() >= UNWATCHED_FOR))
    });
    for (login, _) in unwatched {
        let app = app.clone();
        tokio::spawn(async move { end(&app, login, "nobody is watching").await });
    }
}

/// Ends the visit of `login`, closing its streams saying `why`.
async fn end(app: &Arc<App>, login: LoginKey, why: &'static str) {
    let visit = {
        let mut inner = app.demo_visits.0.lock().unwrap();
        let Some(visit) = inner.visits.remove(&login) else { return };
        // A place already kept for the client's next visit (a takeover) stays with it.
        let leaving = inner.places.get(&visit.client) == Some(&Hold::Visiting(login));
        if leaving {
            inner.places.insert(visit.client, Hold::Leaving);
        }
        (visit, leaving)
    };
    let (visit, leaving) = visit;
    app.auth.lock().unwrap().logout(login);
    app.end_login_with(login, why);
    // A switch under way finishes first (its page opening is bounded); finding the visit gone, it changes nothing.
    let _life = visit.life.lock().await;
    ensure_stopped(app, &visit.session).await;
    visit.gone.send_replace(true);
    tracing::info!(session = %visit.session, "demo visitor done");
    if leaving {
        app.demo_visits.release(visit.client, Hold::Leaving);
    }
}

/// What the client shows a visitor: their session, the demo it shows, and the time left.
pub fn info(app: &App, login: LoginKey) -> Option<Value> {
    let inner = app.demo_visits.0.lock().unwrap();
    inner.visits.get(&login).map(|v| {
        json!({
            "session": v.session,
            "site": v.site,
            "seconds_left": v.deadline.saturating_duration_since(Instant::now()).as_secs(),
        })
    })
}

/// Switches of one visit waiting or under way at once; more are refused (the page asks again with the latest pick).
const MAX_SWITCHING: usize = 2;

/// A switch of a visit: its number, and the lock it takes (switches go one at a time, and the visit's end waits for
/// them). Dropping it ends the switch.
pub struct Switch {
    app: Arc<App>,
    login: LoginKey,
    pub number: u64,
    pub life: Arc<tokio::sync::Mutex<()>>,
}

impl Drop for Switch {
    fn drop(&mut self) {
        if let Some(visit) = self.app.demo_visits.0.lock().unwrap().visits.get_mut(&self.login) {
            visit.switching -= 1;
        }
    }
}

/// Begins a switch of this visit: Err(None) when the visit is over, Err(Some(())) when too many are waiting.
pub fn begin_switch(app: &Arc<App>, login: LoginKey) -> Result<Switch, Option<()>> {
    let mut inner = app.demo_visits.0.lock().unwrap();
    let visit = inner.visits.get_mut(&login).ok_or(None)?;
    if visit.switching >= MAX_SWITCHING {
        return Err(Some(()));
    }
    visit.switching += 1;
    visit.switches += 1;
    Ok(Switch { app: app.clone(), login, number: visit.switches, life: visit.life.clone() })
}

/// One of the visitor's viewer connections to their session, counted from before the WebSocket upgrade until the
/// relay ends, so parallel upgrades cannot get past [`MAX_VIEWERS`]. Counted per session: a relay of an earlier
/// visit that is slow to end never costs the next visitor a place.
pub fn viewer_place(session: &Session) -> Option<crate::app::Place<Arc<std::sync::atomic::AtomicUsize>>> {
    crate::app::Place::take(session.places.clone(), MAX_VIEWERS)
}

/// The visitor's session, while it is their turn.
pub fn active_session(app: &App, login: LoginKey) -> Option<String> {
    app.demo_visits.0.lock().unwrap().visits.get(&login).map(|v| v.session.clone())
}

/// The visitor's browser now shows demo `name` (a switch, in the same session); false when their visit ended meanwhile.
pub fn set_site(app: &App, login: LoginKey, name: String, switch: u64) -> bool {
    let mut inner = app.demo_visits.0.lock().unwrap();
    let Some(visit) = inner.visits.get_mut(&login) else { return false };
    // An older switch whose page opened late does not undo a newer one.
    if switch > visit.shown {
        visit.shown = switch;
        visit.site = name;
    }
    true
}

/// The visitor's deadline, while it is their turn.
pub fn deadline(app: &App, login: LoginKey) -> Option<Instant> {
    app.demo_visits.0.lock().unwrap().visits.get(&login).map(|v| v.deadline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    fn ip(s: &str) -> Client {
        Client::of(s.parse().unwrap())
    }

    fn ready() -> Visits {
        let visits = Visits::default();
        visits.0.lock().unwrap().ready = true;
        visits
    }

    #[test]
    fn a_client_is_an_ipv4_address_or_an_ipv6_64() {
        assert_eq!(ip("203.0.113.7"), ip("::ffff:203.0.113.7"), "a mapped address is its IPv4 one");
        assert_ne!(ip("203.0.113.7"), ip("203.0.113.8"));
        assert_eq!(ip("2001:db8:1:2::1"), ip("2001:db8:1:2:ffff::9"), "one /64");
        assert_ne!(ip("2001:db8:1:2::1"), ip("2001:db8:1:3::1"));
    }

    #[test]
    fn one_place_per_client_and_never_more_than_the_capacity() {
        let visits = Visits::default();
        assert_eq!(visits.reserve(ip("203.0.113.1"), 2), Err(Refused::Opening(5)), "closed until the cleanup");
        let visits = ready();
        assert_eq!(visits.reserve(ip("203.0.113.1"), 2), Ok(()));
        assert!(matches!(visits.reserve(ip("203.0.113.1"), 2), Err(Refused::Yours(_))), "one place per client");
        assert_eq!(visits.reserve(ip("2001:db8::1"), 2), Ok(()));
        assert!(matches!(visits.reserve(ip("203.0.113.2"), 2), Err(Refused::Full(_))));
        visits.release(ip("203.0.113.1"), Hold::Leaving);
        assert!(matches!(visits.reserve(ip("203.0.113.2"), 2), Err(Refused::Full(_))), "a start is not a leave");
        visits.release(ip("203.0.113.1"), Hold::Starting);
        assert_eq!(visits.reserve(ip("203.0.113.2"), 2), Ok(()), "given back");
    }

    #[test]
    fn a_takeover_keeps_the_place_and_counts_as_a_start() {
        let visits = ready();
        let client = ip("203.0.113.9");
        assert!(matches!(visits.take_over(client), Ok(None)), "no visit to take over");
        let old = crate::auth::key_of("old");
        let visit = Visit {
            client,
            session: "s".into(),
            site: "drag".into(),
            deadline: Instant::now(),
            life: Arc::default(),
            gone: tokio::sync::watch::Sender::new(false),
            switches: 0,
            switching: 0,
            shown: 0,
        };
        visits.0.lock().unwrap().visits.insert(old, visit);
        visits.0.lock().unwrap().places.insert(client, Hold::Visiting(old));
        assert!(matches!(visits.take_over(client), Ok(Some((login, _))) if login == old));
        assert_eq!(visits.0.lock().unwrap().places.get(&client), Some(&Hold::Starting), "kept for the new visit");
        assert!(matches!(visits.reserve(ip("203.0.113.10"), 1), Err(Refused::Full(_))), "nobody else takes it");
        // Past the start limit, the open visit stays as it is.
        visits.0.lock().unwrap().places.insert(client, Hold::Visiting(old));
        for _ in 1..MAX_STARTS {
            visits.0.lock().unwrap().starts.get_mut(&client).unwrap().push_back(Instant::now());
        }
        assert!(matches!(visits.take_over(client), Err(Refused::TooOften(_))));
        assert_eq!(visits.0.lock().unwrap().places.get(&client), Some(&Hold::Visiting(old)));
    }

    #[test]
    fn a_client_starting_visits_over_and_over_waits() {
        let visits = ready();
        let client = ip("203.0.113.9");
        for _ in 0..MAX_STARTS {
            assert_eq!(visits.reserve(client, 64), Ok(()));
            visits.release(client, Hold::Starting);
        }
        assert!(matches!(visits.reserve(client, 64), Err(Refused::TooOften(_))));
        assert_eq!(visits.reserve(ip("203.0.113.10"), 64), Ok(()), "another client is not slowed down");
    }

    #[test]
    fn the_demo_configuration_is_strict() {
        assert!(Demo::from_env(None, None).unwrap().is_none());
        let demo = Demo::from_env(some("60"), some("takeover=https://takeover.play/,files=https://files.play/x"))
            .unwrap()
            .unwrap();
        assert_eq!(demo.seconds, 60);
        assert_eq!(demo.start_url("files"), Some("https://files.play/x"));
        assert_eq!(demo.start_url("other"), None);
        for (seconds, sites) in [
            (some("60"), None),
            (None, some("a=https://a.play/")),
            (some("5"), some("a=https://a.play/")),
            (some("6000"), some("a=https://a.play/")),
            (some("60"), some("a=http://a.play/")),
            (some("60"), some("A=https://a.play/")),
            (some("60"), some("a=https://a.play/,a=https://b.play/")),
            (some("60"), some("a")),
            (some("60"), some("a=https://a.play/ x")),
        ] {
            assert!(Demo::from_env(seconds.clone(), sites.clone()).is_err(), "{seconds:?} {sites:?}");
        }
    }
}
