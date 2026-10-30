//! The automation agent of one session: the Cua Driver's MCP server, reached through the controller's `agent_attach`.
//!
//! Calls run one at a time. Taking control first blocks new calls, then waits a bounded time for the in-flight call's
//! reply; without it, the controller kills the agent container and must prove it stopped. Only then is control
//! granted; if no proof comes, the whole session is stopped. Closing the MCP stream alone is never taken as proof.
//! A link that fails is never attached again as it is: the controller gives an agent container one exec, which may
//! still run, so the next call terminates and restarts the agent first.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use line::Line;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Notify, OwnedMutexGuard, Semaphore, SemaphorePermit};

use crate::app::Place;
use crate::assist::{Assist, Route};
use crate::controller::Controller;

/// How long a takeover waits for the in-flight call to answer before killing the agent.
pub const QUIESCENCE: Duration = Duration::from_secs(5);
/// The longest `timeout_ms` a pinned tool's schema allows (`run_script`'s and `get_window_state`'s).
const LONGEST_TIMEOUT_MS: u64 = 120_000;
/// What a call's deadline adds to the longest its tool may take by its own arguments: the driver's work around those
/// budgets (a read's screenshot after its walk's timeout, a batch's actions between its checks).
const CALL_MARGIN: Duration = Duration::from_secs(30);
/// The longest deadline any call gets.
const MAX_CALL_DEADLINE: Duration = Duration::from_secs(600);
/// How long attaching to the agent may take.
const ATTACH_DEADLINE: Duration = Duration::from_secs(120);
/// Calls a session holds at once, per session: the one in flight and up to four waiting for it, or holding a result
/// their caller still scrubs. More are refused as busy, and so is a wait past [`WAIT_DEADLINE`], so callers cannot pile
/// work up behind a slow call. A caller that leaves once its call was admitted gives its place back while the call runs
/// on to its answer as its own task, still holding the link: a session may then hold one call more, whose answer counts
/// against [`ANSWERS`] until it is dropped.
pub const MAX_CALLS: usize = 5;
pub const WAIT_DEADLINE: Duration = Duration::from_secs(30);
/// The longest answer line read from the agent, without its line feed. A screenshot comes as base64 PNG, maybe twice (an image item and
/// `screenshot_png_b64`): at 1920x1080, real pages measured 0.2-0.7 MB of PNG (1.8 MB twice as base64) and a screen of
/// pure noise 6.2 MB (16.6 MB); at the driver's default bound of 1.15 megapixels that noise takes 9.2 MB, and with the
/// walker's 5 000 nodes in both formats the answer 11 MB. A longer one (noise at full size, a script's bulk) fails.
const MAX_LINE: usize = 16 * 1024 * 1024;
/// The most one answer may take once parsed, by [`parsed_size`]: that 11 MB answer takes 42 MiB by it (30 MiB as
/// measured), since an object's B-tree node costs many times its text. A larger one fails.
const MAX_PARSED: usize = 48 * 1024 * 1024;
/// The most one answer may hold while it is read and parsed: its line's buffer (at most [`MAX_LINE`], and half again
/// while it grows), the parser's buffer for an escaped string (three times the longest such string at most, while it
/// grows) and its value. An answer that would take more fails.
const MAX_ANSWER: usize = MAX_LINE * 5 / 2 + MAX_PARSED;
/// What every session's answers may hold at once while they are read, parsed and scrubbed, in KiB (see [`ANSWERS`]):
/// with some 26 MiB the admin takes at rest and one scrub at a time (at most three times the longest string), the
/// admin's 256 MiB keep room for the sessions' own state.
const ANSWERS_KIB: usize = 120 * 1024;
/// The memory agents' answers hold while they are read, parsed and scrubbed, all sessions together. Calls run at once,
/// every session's: nothing is held while the agent works, and only an answer's reading and parsing waits for memory. An
/// answer takes what its reading and parsing take once its line starts, if the link's buffer holds all of the line
/// already (as a short answer comes), else [`MAX_ANSWER`]. It keeps what its reading and parsing may take by its line
/// and [`parsed_size`] while it is parsed, then what its value takes until the scrubbed result or error is serialized,
/// and what the response's bytes take until they are written or their connection is gone (see `service::respond`); an
/// answer never waits for more than it took. A call waits for this holding its agent's link, so the wait ends at once
/// when a takeover or a stop blocks the agent (the call then fails as blocked and leaves its link lost, see
/// [`Gate::share`]), and at the call's deadline.
static ANSWERS: Budget = Budget::new(ANSWERS_KIB);
const _: () = assert!(MAX_ANSWER <= ANSWERS_KIB * 1024, "an answer must fit");

/// A budget of memory, in KiB, that answers take shares of (see [`ANSWERS`]). A share is taken only once all of it is
/// free, never piece by piece as memory is given back, so a share that waits for much never holds back a smaller one
/// that fits (a short answer never waits behind others' large ones); one that waits may see smaller ones go first.
pub struct Budget {
    free: Semaphore,
    /// Wakes the shares waiting for memory once some is given back.
    given: Notify,
}

impl Budget {
    pub const fn new(kib: usize) -> Budget {
        Budget { free: Semaphore::const_new(kib), given: Notify::const_new() }
    }

    /// A share of `kib` KiB, if that much is free.
    pub fn take(&'static self, kib: usize) -> Option<Held> {
        let permit = self.free.try_acquire_many(u32::try_from(kib).ok()?).ok()?;
        Some(Held { permit, budget: self })
    }

    /// What is free, in KiB.
    #[cfg(test)]
    pub fn free(&self) -> usize {
        self.free.available_permits()
    }
}

/// An answer's share of a [`Budget`], given back when it is dropped.
pub struct Held {
    permit: SemaphorePermit<'static>,
    budget: &'static Budget,
}

impl Held {
    /// What it holds, in KiB.
    pub fn kib(&self) -> usize {
        self.permit.num_permits()
    }

    /// Gives back all of it but `kib` KiB, if it holds more.
    fn keep(&mut self, kib: usize) {
        let given = self.kib().saturating_sub(kib);
        if given > 0 {
            drop(self.permit.split(given));
            self.budget.given.notify_waiters();
        }
    }

    /// An answer's share made to cover `bytes`, what its value takes as JSON once scrubbed: redaction may lengthen it
    /// past its share (a `[redacted]` can be longer than what it replaces), and the difference is taken from the same
    /// budget at once. None when the difference is not free: the share is given back, and the value must go too. Never
    /// by waiting: an answer that waited for more while it holds its share could wait behind another answer's share,
    /// which waits for what this one holds.
    pub fn grown(mut self, bytes: usize) -> Option<Held> {
        let more = kib(bytes).saturating_sub(self.kib());
        if more > 0 {
            let more = self.budget.free.try_acquire_many(u32::try_from(more).ok()?).ok()?;
            self.permit.merge(more);
        }
        Some(self)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // Given back before the waiting shares wake to look.
        self.keep(0);
    }
}

/// `bytes` in KiB, as [`ANSWERS`] counts them.
fn kib(bytes: usize) -> usize {
    bytes.div_ceil(1024)
}

/// An answer's share brought down to what `bytes` bytes take, once its value is serialized into them and dropped: never
/// more than it held, which covers its value as JSON once scrubbed (see [`Held::grown`]), so only what the response
/// wraps around that value (a field's name, a JSON-RPC id its caller chose) may take more.
pub fn serialized(mut held: Held, bytes: usize) -> Held {
    held.keep(kib(bytes));
    held
}

/// What an agent's calls go through: whether a takeover (or a stop) blocks them, and the budget their answers take from.
struct Gate {
    /// [`OPEN`], [`BLOCKED`], or [`STOPPED`]: blocked for good, which nothing unblocks.
    blocked: AtomicU8,
    /// Wakes the calls waiting for their answer's share once the agent is blocked.
    closed: Notify,
    /// [`ANSWERS`], but in tests.
    answers: &'static Budget,
}

const OPEN: u8 = 0;
const BLOCKED: u8 = 1;
const STOPPED: u8 = 2;

impl Gate {
    fn new(answers: &'static Budget) -> Gate {
        Gate { blocked: AtomicU8::new(OPEN), closed: Notify::new(), answers }
    }

    fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::SeqCst) != OPEN
    }

    fn block(&self) {
        self.blocked.fetch_max(BLOCKED, Ordering::SeqCst);
        self.closed.notify_waiters();
    }

    /// Blocks it for good: a session being stopped. A resume or give-back still under way never unblocks it after.
    fn stop(&self) {
        self.blocked.store(STOPPED, Ordering::SeqCst);
        self.closed.notify_waiters();
    }

    fn unblock(&self) {
        let _ = self.blocked.compare_exchange(BLOCKED, OPEN, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// An answer's share of the budget, `bytes` (at most [`MAX_ANSWER`]): at once when it is free, the call in flight's
    /// answer included while a takeover waits for it; otherwise by waiting, which a block ends with none (the call
    /// waiting holds the link, which a takeover must get back at once).
    async fn share(&self, bytes: usize) -> Option<Held> {
        let size = kib(bytes.min(MAX_ANSWER));
        if let Some(share) = self.answers.take(size) {
            return Some(share);
        }
        let closed = self.closed.notified();
        tokio::pin!(closed);
        closed.as_mut().enable();
        loop {
            let given = self.answers.given.notified();
            tokio::pin!(given);
            // Waiting from here on: a block after this check wakes it, and so does memory given back after the look.
            given.as_mut().enable();
            if self.is_blocked() {
                return None;
            }
            if let Some(share) = self.answers.take(size) {
                return Some(share);
            }
            // A block that came while it waited wins over memory given back at the same time.
            tokio::select! {
                biased;
                () = closed.as_mut() => return None,
                () = given => {}
            }
        }
    }
}

struct Link {
    stream: BufReader<UnixStream>,
    next_id: u64,
}

/// The link to the agent as the next call finds it.
enum State {
    /// None yet, to a fresh (or restarted) agent: the next call attaches.
    Fresh,
    Open(Link),
    /// Failed, dropped mid-call, or never fully attached: its exec may still run, and the controller gives an agent
    /// container one exec, so the next call terminates and restarts the agent before it attaches again.
    Lost,
}

pub struct Agent {
    session: String,
    controller: Controller,
    /// Held for the whole of a call, so holding it proves no call is in flight.
    link: Arc<Mutex<State>>,
    gate: Arc<Gate>,
    /// Calls on this session now: in flight, waiting for the link, or holding a result their caller still works on
    /// (see [`MAX_CALLS`]). Each is placed under `activity`'s lock.
    waiting: std::sync::atomic::AtomicUsize,
    /// What [`Agent::claim_if_idle`] decides on. Its lock is taken last (under a session's state at most) and waits for
    /// nothing under it: the link is only tried there.
    activity: Arc<std::sync::Mutex<Activity>>,
    /// What the admin does around the driver's calls on this display (see `assist`).
    assist: Arc<Assist>,
}

/// How an agent uses its session, besides the calls placed.
#[derive(Default)]
struct Activity {
    /// Admitted calls still running, also those whose caller left (each runs on as its own task).
    running: usize,
    /// When the agent last used the session: a call's end (a refused one too), or its start of the session.
    last: Option<Instant>,
}

/// An admitted call running, counted in [`Activity::running`] until it is dropped, after its exchange ended: the end
/// is recorded before the call stops counting.
struct Running(Arc<std::sync::Mutex<Activity>>);

impl Running {
    fn new(activity: Arc<std::sync::Mutex<Activity>>) -> Running {
        activity.lock().unwrap().running += 1;
        Running(activity)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut activity = self.0.lock().unwrap();
        activity.running -= 1;
        activity.last = Some(Instant::now());
    }
}

#[derive(Debug)]
pub enum CallError {
    /// A person holds control (or a takeover is under way).
    Blocked,
    /// Too many calls wait for this agent already, or this one waited [`WAIT_DEADLINE`]: it was asked nothing.
    Busy,
    /// The caller may no longer call (see [`Agent::call`]).
    Withdrawn,
    Failed(String),
}

pub enum Quiescence {
    /// The in-flight call (if any) answered; the agent still runs, idle.
    Idle,
    /// The agent did not answer in time and its container was proven stopped.
    Terminated,
}

impl Agent {
    /// The agent of `session`, whose display's stream socket is `display`.
    pub fn new(session: String, controller: Controller, display: std::path::PathBuf) -> Agent {
        Agent::on(session, controller, display, &ANSWERS)
    }

    /// An agent whose answers take from `answers`: [`ANSWERS`], but in tests.
    pub fn on(session: String, controller: Controller, display: std::path::PathBuf, answers: &'static Budget) -> Agent {
        Agent {
            session,
            controller,
            link: Arc::new(Mutex::new(State::Fresh)),
            gate: Arc::new(Gate::new(answers)),
            waiting: std::sync::atomic::AtomicUsize::new(0),
            activity: Arc::default(),
            assist: Arc::new(Assist::new(display)),
        }
    }

    /// Where the agent acts, in output pixels, for the session's viewers (see `assist`).
    pub fn places(&self) -> tokio::sync::watch::Receiver<Option<(u16, u16)>> {
        self.assist.places()
    }

    pub fn is_blocked(&self) -> bool {
        self.gate.is_blocked()
    }

    /// Records that the agent used the session just now (it started it).
    pub fn used_now(&self) {
        self.activity.lock().unwrap().last = Some(Instant::now());
    }

    /// Blocks new calls if no agent is using the session, and says whether it did: no call placed or running, none
    /// ended (refused ones included: that agent is trying) less than `recent` ago, and a link ready for the next call
    /// (a lost one needs the agent killed first, which this never does). Calls are placed under the same lock, so a
    /// call either counts here or finds the agent blocked; once blocked, the agent is quiescent at once, with no wait.
    /// The link is only tried (`try_lock`), under the activity lock, while an admitted call takes that lock holding
    /// the link (`Running::new`): neither waits for the other.
    pub fn claim_if_idle(&self, recent: Duration) -> bool {
        let activity = self.activity.lock().unwrap();
        let unused = self.waiting.load(Ordering::SeqCst) == 0
            && activity.running == 0
            && activity.last.is_none_or(|at| at.elapsed() >= recent);
        // A link held while nothing is counted is a takeover's or a stop's.
        let ready = unused && matches!(self.link.try_lock().as_deref(), Ok(State::Fresh | State::Open(_)));
        if ready {
            self.gate.block();
        }
        ready
    }

    /// A place for one call on this session, or none when [`MAX_CALLS`] calls are waiting or holding their result
    /// already. The caller keeps it until it is done with the call's result or error (scrubbed it), so a session holds
    /// a bounded number of them however many calls follow; it is given back when dropped.
    pub fn place(&self) -> Option<Turn<'_>> {
        let _activity = self.activity.lock().unwrap();
        let place = Place::take(&self.waiting, MAX_CALLS)?;
        Some(Turn { _place: place, answer: None, activity: &self.activity })
    }

    /// Runs one tool and returns its MCP result. Waiting for the previous call is the caller's (a caller that leaves
    /// before its turn asks nothing of the agent); once admitted, the exchange runs as its own task holding the link,
    /// so a caller that disconnects never cuts it half way. `still` says whether the caller may still call once it is
    /// admitted (a token revoked meanwhile may not). What its answer holds of [`ANSWERS`] is left in `turn`.
    pub async fn call(
        &self,
        turn: &mut Turn<'_>,
        tool: &str,
        arguments: Value,
        still: impl FnOnce() -> bool,
    ) -> Result<Value, CallError> {
        if self.is_blocked() {
            return Err(CallError::Blocked);
        }
        // Past the deadline the wait is dropped: nothing stays queued. A caller that leaves while it waits gives its
        // place back too (the place is dropped either way).
        let guard =
            tokio::time::timeout(WAIT_DEADLINE, self.link.clone().lock_owned()).await.map_err(|_| CallError::Busy)?;
        // A takeover may have started while this call waited for the previous one.
        if self.is_blocked() {
            return Err(CallError::Blocked);
        }
        if !still() {
            return Err(CallError::Withdrawn);
        }
        let (controller, session, gate) = (self.controller.clone(), self.session.clone(), self.gate.clone());
        let call = Call::new(tool, arguments, self.assist.clone());
        let running = Running::new(self.activity.clone());
        let exchange = async move {
            let ended = run(guard, controller, session, gate, call).await;
            drop(running);
            ended
        };
        let (answer, result) = tokio::spawn(exchange)
            .await
            .unwrap_or_else(|_| (None, Err(CallError::Failed("the agent call failed".into()))));
        turn.answer = answer;
        result
    }

    /// Blocks new calls and proves the agent is quiescent: the in-flight call answered in time, or the agent's
    /// container was killed and proven stopped. An error means neither proof came and the session must be stopped.
    pub async fn quiesce(&self) -> Result<Quiescence, String> {
        self.gate.block();
        match tokio::time::timeout(QUIESCENCE, self.link.lock()).await {
            // A lost link's exec may still be running its tool: only a proven stop makes the agent quiescent.
            Ok(state) if matches!(*state, State::Lost) => {
                drop(state);
                tracing::warn!(session = %self.session, "the agent's link was lost; terminating it before control");
                self.controller.agent_terminate(&self.session).await?;
                Ok(Quiescence::Terminated)
            }
            Ok(_idle) => Ok(Quiescence::Idle),
            Err(_) => {
                tracing::warn!(session = %self.session, "the agent did not answer within the bound; terminating it");
                self.controller.agent_terminate(&self.session).await?;
                // The in-flight call now fails on the closed stream and releases the link.
                let mut guard = tokio::time::timeout(QUIESCENCE, self.link.lock())
                    .await
                    .map_err(|_| "the terminated agent's call never ended".to_owned())?;
                *guard = State::Lost;
                Ok(Quiescence::Terminated)
            }
        }
    }

    /// Blocks new calls for good (a resume or give-back still under way never unblocks them) and closes an idle link, so
    /// a session being stopped holds no controller connection (each link holds one); a call in flight drops its link
    /// itself once the stopped agent's stream ends.
    pub fn detach(&self) {
        self.gate.stop();
        if let Ok(mut link) = self.link.try_lock() {
            *link = State::Lost;
        }
    }

    /// Lets the agent act again. A terminated agent is restarted fresh; the next call attaches to it. It is terminated
    /// again first (a no-op when it is): a lost-link call that was restarting it when the takeover terminated it may
    /// have started a fresh one since, which the controller refuses to restart.
    pub async fn resume(&self, terminated: bool) -> Result<(), String> {
        if terminated {
            self.controller.agent_terminate(&self.session).await?;
            self.controller.agent_restart(&self.session).await?;
            *self.link.lock().await = State::Fresh;
        }
        self.gate.unblock();
        Ok(())
    }
}

/// When a call is treated as stuck: past the longest its tool may take by the call's own arguments (each bounded by
/// its schema's maximum, or by the longest pinned timeout where the schema sets none), and never before the longest
/// pinned timeout, plus [`CALL_MARGIN`], and at most [`MAX_CALL_DEADLINE`]. A batch (`run_steps`) may take each
/// step's target wait, its `wait_for` and `expect` checks, the pauses between steps and its closing read.
fn deadline(tool: &str, arguments: &Value) -> Duration {
    let ms = |value: Option<&Value>, default: u64, max: u64| value.and_then(Value::as_u64).unwrap_or(default).min(max);
    let longest = LONGEST_TIMEOUT_MS;
    let own = match tool {
        "run_steps" => {
            let steps = arguments.get("steps").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            let step = |step: &Value| {
                let wait = step.get("wait_for").map_or(0, |w| ms(w.get("timeout_ms"), 5_000, longest));
                let expect = step.get("expect").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
                let checks: u64 = expect.iter().map(|e| ms(e.get("timeout_ms"), 2_000, longest)).sum();
                ms(step.get("timeout_ms"), 3_000, 10_000) + wait + checks
            };
            let pauses = ms(arguments.get("delay_ms"), 0, 2_000) * steps.len().saturating_sub(1) as u64;
            let observe = arguments.get("observe").map_or(0, |o| ms(o.get("timeout_ms"), 1_000, longest));
            steps.iter().map(step).fold(pauses + observe, u64::saturating_add)
        }
        _ => ms(arguments.get("timeout_ms"), longest, longest),
    };
    (Duration::from_millis(own.max(longest)) + CALL_MARGIN).min(MAX_CALL_DEADLINE)
}

/// A call's place (see [`Agent::place`]): kept with its result or error while the caller still works on it, while the
/// agent's link stays free for the next call and a takeover.
pub struct Turn<'a> {
    _place: Place<&'a std::sync::atomic::AtomicUsize>,
    /// What its answer takes, result or error (see [`ANSWERS`]).
    answer: Option<Held>,
    activity: &'a std::sync::Mutex<Activity>,
}

impl Drop for Turn<'_> {
    /// Records the call's end before its place is given back (the fields drop after this): a refused call is the agent
    /// using the session too.
    fn drop(&mut self) {
        self.activity.lock().unwrap().last = Some(Instant::now());
    }
}

impl Turn<'_> {
    /// What the call's answer holds of [`ANSWERS`], to give back with the response made of it (see
    /// `service::respond`).
    pub fn answer(&mut self) -> Option<Held> {
        self.answer.take()
    }
}

/// One call to the driver: its tool and arguments as they go to it, and what the admin does around it (see `assist`).
struct Call {
    tool: String,
    arguments: Value,
    route: Route,
    /// The arguments as the agent sent them, for a tool the admin needs them of (see `assist::needs_arguments`).
    sent: Option<Value>,
    assist: Arc<Assist>,
}

impl Call {
    fn new(tool: &str, mut arguments: Value, assist: Arc<Assist>) -> Call {
        let sent = crate::assist::needs_arguments(tool).then(|| arguments.clone());
        let route = assist.route(tool, &mut arguments);
        Call { tool: tool.to_owned(), arguments, route, sent, assist }
    }
}

/// One admitted call, holding the link: attaches (restarting a lost agent first), asks, and puts the link back only once
/// the agent answered, so a call that fails, is blocked while its answer waits for memory, or is dropped, leaves it lost.
/// What the admin does around the answer is done holding the link too. The agent's answer comes with what it holds of
/// [`ANSWERS`].
async fn run(
    mut guard: OwnedMutexGuard<State>,
    controller: Controller,
    session: String,
    gate: Arc<Gate>,
    call: Call,
) -> (Option<Held>, Result<Value, CallError>) {
    let Call { tool, arguments, route, sent, assist } = call;
    // Answered without the driver: nothing is asked of it, and its link stays as it is.
    if let Some(refusal) = route.refusal() {
        return (None, Ok(refusal));
    }
    let attached = match std::mem::replace(&mut *guard, State::Lost) {
        State::Open(link) => Ok(link),
        State::Fresh => attach(&controller, &session, &gate).await,
        State::Lost => restart(&controller, &session, &gate).await,
    };
    let mut link = match attached {
        Ok(link) => link,
        Err(e) => return (None, Err(e)),
    };
    // A takeover may have started while this call attached; nothing was asked of the agent yet.
    if gate.is_blocked() {
        *guard = State::Open(link);
        return (None, Err(CallError::Blocked));
    }
    let id = link.next_id;
    link.next_id += 1;
    let deadline = deadline(&tool, &arguments);
    let request =
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
    match tokio::time::timeout(deadline, exchange(&mut link, &gate, request, id)).await {
        Ok(Ok((mut response, held))) => {
            *guard = State::Open(link);
            // Moved out, not copied: a result may be tens of MiB.
            let result = match response.as_object_mut().and_then(|m| m.remove("result")) {
                Some(result) => assist.answered(|| gate.is_blocked(), &tool, sent.as_ref(), route, result).await,
                None => Err(CallError::Failed(response.get("error").map_or("no result".into(), Value::to_string))),
            };
            (Some(held), result)
        }
        Ok(Err(e)) => (None, Err(e)),
        Err(_) => (None, Err(CallError::Failed("the agent did not answer in time".into()))),
    }
}

/// Terminates and restarts an agent whose link was lost, then attaches to it.
async fn restart(controller: &Controller, session: &str, gate: &Gate) -> Result<Link, CallError> {
    tracing::warn!(%session, "the agent's link was lost; restarting the agent");
    controller.agent_terminate(session).await.map_err(CallError::Failed)?;
    // A takeover that started meanwhile wants the agent stopped: no fresh one while a person holds control. The link
    // stays lost, so the give-back's restart (or the next call) starts it.
    if gate.is_blocked() {
        return Err(CallError::Blocked);
    }
    controller.agent_restart(session).await.map_err(CallError::Failed)?;
    attach(controller, session, gate).await
}

async fn attach(controller: &Controller, session: &str, gate: &Gate) -> Result<Link, CallError> {
    let stream = controller.agent_attach(session).await.map_err(CallError::Failed)?;
    let mut link = Link { stream, next_id: 1 };
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "easi-run-admin", "version": env!("CARGO_PKG_VERSION")}}});
    // Its answer is let go at once.
    let _ = tokio::time::timeout(ATTACH_DEADLINE, exchange(&mut link, gate, init, 0))
        .await
        .map_err(|_| CallError::Failed("the agent did not initialize in time".into()))??;
    send(&mut link, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await
        .map_err(CallError::Failed)?;
    Ok(link)
}

async fn send(link: &mut Link, message: &Value) -> Result<(), String> {
    let mut line = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    line.push(b'\n');
    link.stream.get_mut().write_all(&line).await.map_err(|e| format!("agent: {e}"))
}

/// Sends one request and reads lines until its response, skipping notifications and other ids, and returns it with
/// what it holds of [`ANSWERS`]. A line longer than [`MAX_LINE`] fails the exchange at once (the rest of it could never
/// be read as an answer), and so does one that would take more than [`MAX_PARSED`] parsed or [`MAX_ANSWER`] in all, and
/// one nested deeper than the parser follows: the driver writes only JSON-RPC lines, so that one is most likely the
/// answer, which would otherwise be waited for until the call's deadline. It is blocked once the agent is, while a line
/// waits for its share (see [`Gate::share`]).
async fn exchange(link: &mut Link, gate: &Gate, request: Value, id: u64) -> Result<(Value, Held), CallError> {
    let failed = |e: std::io::Error| CallError::Failed(format!("agent: {e}"));
    let deep = || CallError::Failed(format!("the agent's answer nests deeper than {PARSER_DEPTH} levels"));
    send(link, &request).await.map_err(CallError::Failed)?;
    loop {
        // Taken once the line starts, and before any of it is read: what the line takes if it is all in the buffer.
        let buffered = link.stream.fill_buf().await.map_err(failed)?;
        if buffered.is_empty() {
            return Err(CallError::Failed("the agent's stream closed".into()));
        }
        let whole = buffered.iter().position(|&byte| byte == b'\n').map(|end| &buffered[..end]);
        let size = match whole.and_then(|line| std::str::from_utf8(line).ok()) {
            Some(line) => taken(line).ok_or_else(deep)?.0,
            None => MAX_ANSWER,
        };
        let mut held = gate.share(size).await.ok_or(CallError::Blocked)?;
        let line = match line::read(&mut link.stream, MAX_LINE).await.map_err(failed)? {
            Line::Complete(line) => {
                String::from_utf8(line).map_err(|_| CallError::Failed("agent: an answer that is not UTF-8".into()))?
            }
            Line::TooLong => {
                return Err(CallError::Failed(format!("the agent's answer is larger than {} MiB", MAX_LINE >> 20)));
            }
            Line::Closed => return Err(CallError::Failed("the agent's stream closed".into())),
        };
        let (parsing, size) = taken(&line).ok_or_else(deep)?;
        if size > MAX_PARSED {
            let why = format!("the agent's answer would take more than {} MiB parsed", MAX_PARSED >> 20);
            return Err(CallError::Failed(why));
        }
        if parsing > MAX_ANSWER {
            let why = format!("the agent's answer would take more than {} MiB to parse", MAX_ANSWER >> 20);
            return Err(CallError::Failed(why));
        }
        held.keep(kib(parsing));
        let Some(message) = parse(&line) else { continue };
        drop(line);
        held.keep(kib(size));
        if message.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok((message, held));
        }
    }
}

/// What reading and parsing `line` takes: its buffer, as [`line::read`] grows it, the parser's buffer for its longest
/// escaped string (three times that at most, while it grows) and its value, by [`parsed_size`]; with what its value
/// alone takes. None if the parser refuses it.
fn taken(line: &str) -> Option<(usize, usize)> {
    let (size, escaped) = parsed_size(line)?;
    Some((line.len().next_power_of_two().min(MAX_LINE) + 3 * escaped + size, size))
}

/// `line` as a JSON value, its object keys taken as written: `serde_json`'s own [`Value`] parses again the string under
/// one reserved key once its `raw_value` feature is on (axum turns it on), which [`parsed_size`] does not count.
fn parse(line: &str) -> Option<Value> {
    let mut parser = serde_json::Deserializer::from_str(line);
    let value = serde::de::DeserializeSeed::deserialize(Literal, &mut parser).ok()?;
    parser.end().ok()?;
    Some(value)
}

/// What [`parse`] builds: a [`Value`] as `serde_json`'s own would be, but for that reserved key.
struct Literal;

impl<'de> serde::de::DeserializeSeed<'de> for Literal {
    type Value = Value;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Literal {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut items: A) -> Result<Value, A::Error> {
        let mut array = Vec::new();
        while let Some(item) = items.next_element_seed(Literal)? {
            array.push(item);
        }
        Ok(Value::Array(array))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut object = serde_json::Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            object.insert(key, entries.next_value_seed(Literal)?);
        }
        Ok(Value::Object(object))
    }
}

/// The deepest nesting `serde_json` parses.
const PARSER_DEPTH: usize = 127;

/// The most parsing `line` into a [`Value`] allocates, from its structure alone: each string's bytes (no more than as
/// written) and their allocation, each array's buffer as it grows by doubling (the one it outgrew still held while it
/// is copied), and each object's B-tree nodes (one of eleven entries, and past those a node at least five entries full
/// per five, with their parents). Measured over arrays, objects and strings of many shapes and sizes, as malloc rounds
/// each allocation, it came to 1.0-1.9 times the parser's peak. With it, the longest string with an escape as written,
/// which the parser decodes in a buffer of its own. None if the line nests deeper than the parser follows: the parser
/// refuses it.
fn parsed_size(line: &str) -> Option<(usize, usize)> {
    /// What an allocation takes beyond the bytes asked for, rounded.
    const ALLOCATION: usize = 32;
    /// A value in an array's buffer, half again while the buffer grows.
    const SLOT: usize = 48;
    /// A B-tree leaf, which holds up to eleven entries.
    const LEAF: usize = 648;
    /// Past eleven entries: each entry's share of leaves five entries full and of their parents, and the nodes above.
    const ENTRY: usize = 160;
    const TREE: usize = 2168;
    // Each open array or object: whether it is an object, its entries (or its commas), and whether it holds a value.
    let mut open: Vec<(bool, usize, bool)> = Vec::new();
    let mut size = line.len() + ALLOCATION;
    let close = |(object, items, any): (bool, usize, bool)| match (object, items) {
        (true, 0) => 0,
        (true, 1..=11) => LEAF,
        (true, _) => TREE + items * ENTRY,
        (false, _) if !any => 0,
        (false, _) => (items + 1).next_power_of_two().max(4) * SLOT + ALLOCATION,
    };
    let (mut string, mut escaped) = (false, false);
    // Where the open string starts and whether it holds an escape, and the longest one that did.
    let (mut start, mut escapes, mut longest) = (0, false, 0);
    for (at, &byte) in line.as_bytes().iter().enumerate() {
        if string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => (escaped, escapes) = (true, true),
                b'"' => {
                    string = false;
                    if escapes {
                        longest = longest.max(at - start);
                    }
                }
                _ => {}
            }
            continue;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        if let Some(last) = open.last_mut()
            && !matches!(byte, b']' | b'}')
        {
            last.2 = true;
            match byte {
                b',' if !last.0 => last.1 += 1,
                b':' if last.0 => last.1 += 1,
                _ => {}
            }
        }
        match byte {
            b'"' => {
                (string, start, escapes) = (true, at, false);
                size += ALLOCATION;
            }
            b'[' | b'{' if open.len() == PARSER_DEPTH => return None,
            b'[' | b'{' => open.push((byte == b'{', 0, false)),
            b']' | b'}' => size += open.pop().map_or(0, close),
            _ => {}
        }
    }
    // A line cut short is parsed as far as it goes.
    if string && escapes {
        longest = longest.max(line.len() - start);
    }
    Some((size + open.into_iter().map(close).sum::<usize>(), longest))
}

#[cfg(test)]
pub(crate) mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    /// The largest `timeout_ms` (and `delay_ms`) each pinned schema allows, wherever it sits.
    fn maxima(schema: &Value, out: &mut Vec<u64>) {
        match schema {
            Value::Object(map) => {
                for (key, value) in map {
                    if key == "timeout_ms"
                        && let Some(max) = value.get("maximum").and_then(Value::as_u64)
                    {
                        out.push(max);
                    }
                    maxima(value, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| maxima(item, out)),
            _ => {}
        }
    }

    #[test]
    fn a_call_s_deadline_exceeds_the_longest_its_own_arguments_allow() {
        let mut found = Vec::new();
        crate::tools::driver().iter().for_each(|tool| maxima(&tool["inputSchema"], &mut found));
        assert_eq!(found.iter().max(), Some(&LONGEST_TIMEOUT_MS), "{found:?}");
        let longest = Duration::from_millis(LONGEST_TIMEOUT_MS);
        for (tool, arguments) in [
            ("run_script", json!({"script": "return 1;", "timeout_ms": 120_000})),
            ("get_window_state", json!({"pid": 1, "window_id": 2, "timeout_ms": 120_000})),
            ("get_window_state", json!({"pid": 1, "window_id": 2, "timeout_ms": 99_999_999})),
            ("click", json!({"x": 1, "y": 2})),
        ] {
            assert!(deadline(tool, &arguments) >= longest + CALL_MARGIN, "{tool} {arguments}");
        }
        // A long batch: 64 steps, each at its longest target wait, a wait and four checks, with pauses between.
        let step = json!({"tool": "click", "args": {}, "timeout_ms": 10_000,
            "wait_for": {"text": "a"}, "expect": [{"text": "a"}, {"text": "b"}, {"text": "c"}, {"text": "d"}]});
        let batch = json!({"steps": vec![step; 16], "delay_ms": 2_000, "observe": {"timeout_ms": 5_000}});
        let own = Duration::from_millis(16 * (10_000 + 5_000 + 4 * 2_000) + 15 * 2_000 + 5_000);
        assert_eq!(deadline("run_steps", &batch), own + CALL_MARGIN);
        // Never more than the bound, whatever the arguments ask.
        let step = json!({"tool": "click", "args": {}, "expect": vec![json!({"timeout_ms": u64::MAX}); 4]});
        assert_eq!(deadline("run_steps", &json!({"steps": vec![step; 64]})), MAX_CALL_DEADLINE);
    }

    #[tokio::test]
    async fn an_answer_longer_than_a_line_may_be_fails_at_once() {
        let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {"text": "a".repeat(MAX_LINE)}});
        let outcome = tokio::time::timeout(Duration::from_secs(30), answer(format!("{reply}\n"))).await;
        assert_eq!(outcome.expect("answered at once").err(), Some("the agent's answer is larger than 16 MiB".into()));
    }

    /// The agent's end of a link that reads one request, writes `reply`, and stays open.
    fn peer(reply: String) -> UnixStream {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        tokio::spawn(async move {
            let mut request = vec![0u8; 256];
            let _ = theirs.read(&mut request).await;
            theirs.write_all(reply.as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
        ours
    }

    /// The agent's answer to one request, as `exchange` takes it. The stream stays open after `reply`: only what it
    /// holds can end the exchange.
    async fn answer(reply: String) -> Result<(Value, Held), String> {
        let mut link = Link { stream: BufReader::new(peer(reply)), next_id: 1 };
        let gate = Gate::new(budget());
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call"});
        exchange(&mut link, &gate, request, 1).await.map_err(|e| match e {
            CallError::Failed(why) => why,
            other => format!("{other:?}"),
        })
    }

    #[tokio::test]
    async fn an_answer_that_would_take_too_much_parsed_fails_and_one_that_fits_holds_its_share() {
        // Two MiB of zeros, 32 bytes each as values.
        let zeros = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[{}0]}}\n", "0,".repeat(1 << 20));
        assert_eq!(answer(zeros).await.err(), Some("the agent's answer would take more than 48 MiB parsed".to_owned()));
        // A screenshot of noise twice, as base64, beside a tree of the walker's 5 000 nodes in both formats.
        let shot = "A".repeat(4_600_000);
        let element = |n: usize| {
            json!({"element_index": n, "element_token": format!("s0000002a:{n}"), "role": "push button",
                "label": format!("Button {n}"), "value": "", "enabled": true, "selected": false, "actions": ["click"],
                "frame": {"x": 100, "y": 200, "w": 80, "h": 30}, "screenshot_frame": {"x": 70, "y": 140, "w": 56, "h": 21},
                "frame_scale": 0.7, "parent_index": n / 2, "depth": 12})
        };
        let markdown = (0..5_000).map(|n| format!("- [{n}] push button \"Button {n}\"\n")).collect::<String>();
        let result = json!({"content": [{"type": "text", "text": markdown}, {"type": "image", "data": shot,
            "mimeType": "image/png"}], "structuredContent": {"elements": (0..5_000).map(element).collect::<Vec<_>>(),
            "tree_markdown": markdown, "screenshot_png_b64": shot}});
        let reply = format!("{}\n", json!({"jsonrpc": "2.0", "id": 1, "result": result}));
        let size = parsed_size(&reply).unwrap().0;
        assert!(size < MAX_PARSED, "{} MiB", size >> 20);
        let (message, held) = answer(reply).await.unwrap();
        assert_eq!(message["result"], result);
        assert_eq!(held.kib(), size.div_ceil(1024));
    }

    #[test]
    fn an_answer_grows_by_what_is_free_at_once_or_not_at_all() {
        let answers: &'static Budget = Box::leak(Box::new(Budget::new(100)));
        let held = answers.take(10).unwrap().grown(30 * 1024).unwrap();
        assert_eq!((held.kib(), answers.free()), (30, 70), "the difference merged into the share");
        let held = held.grown(1024).unwrap();
        assert_eq!(held.kib(), 30, "never brought down here");
        assert!(held.grown(101 * 1024).is_none());
        assert_eq!(answers.free(), 100, "a share that cannot grow goes back");
    }

    #[test]
    fn an_answer_is_parsed_with_its_keys_as_written() {
        let line = r#"{"$serde_json::private::RawValue":"[0,0]"}"#;
        assert!(serde_json::from_str::<Value>(line).unwrap().is_array(), "serde_json's own parses it again");
        assert_eq!(parse(line), Some(json!({"$serde_json::private::RawValue": "[0,0]"})));
        assert_eq!(
            parse("[1, -2, 3.5, true, null, \"a\\nb\", {\"k\": {}}]"),
            serde_json::from_str("[1, -2, 3.5, true, null, \"a\\nb\", {\"k\": {}}]").ok()
        );
        assert_eq!(parse("[1] x"), None);
    }

    #[tokio::test]
    async fn an_escaped_string_counts_the_parser_s_buffer_for_it() {
        let text = format!("{}\\n", "a".repeat(12 << 20));
        let line = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"{text}\"}}\n");
        assert!(parsed_size(&line).unwrap().1 > text.len());
        // Its 16 MiB buffer, the parser's three times the string, and its value (under the most a value may take): more
        // than an answer may hold.
        let line = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":[\"{text}\",{}0]}}\n", "0,".repeat(1 << 18));
        assert!(parsed_size(&line).unwrap().0 < MAX_PARSED);
        assert_eq!(
            answer(line).await.err(),
            Some("the agent's answer would take more than 88 MiB to parse".to_owned())
        );
    }

    #[tokio::test]
    async fn an_answer_nested_deeper_than_the_parser_follows_fails_at_once() {
        let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let why = format!("the agent's answer nests deeper than {PARSER_DEPTH} levels");
        // All in the buffer, and longer than it.
        for padding in [0, 64 * 1024] {
            let value = format!("{{\"value\":{},\"pad\":\"{}\"}}", nested(PARSER_DEPTH + 3), "a".repeat(padding));
            let reply = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"structuredContent\":{value}}}}}\n");
            let outcome = tokio::time::timeout(Duration::from_secs(10), answer(reply)).await;
            assert_eq!(outcome.expect("failed at once").err(), Some(why.clone()), "padding {padding}");
        }
    }

    #[test]
    fn a_line_nested_deeper_than_the_parser_follows_has_no_size() {
        let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(parsed_size(&nested(PARSER_DEPTH + 1)).is_none());
        assert!(serde_json::from_str::<Value>(&nested(PARSER_DEPTH + 1)).is_err());
        assert!(parsed_size(&nested(PARSER_DEPTH)).is_some());
        assert!(serde_json::from_str::<Value>(&nested(PARSER_DEPTH)).is_ok());
    }

    #[tokio::test]
    async fn detaching_closes_an_idle_link_and_blocks_new_calls() {
        let agent = agent(Controller::new("/nonexistent".into()));
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        *agent.link.lock().await = State::Open(Link { stream: BufReader::new(ours), next_id: 1 });
        agent.detach();
        assert!(matches!(*agent.link.lock().await, State::Lost));
        let mut rest = Vec::new();
        assert_eq!(theirs.read_to_end(&mut rest).await.unwrap(), 0, "the controller sees the link close");
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Blocked)));
    }

    /// A budget as large as [`ANSWERS`], for one test alone: tests that run at once never wait for each other's.
    fn budget() -> &'static Budget {
        Box::leak(Box::new(Budget::new(ANSWERS_KIB)))
    }

    /// An agent whose answers take from `answers`.
    fn agent_on(controller: Controller, answers: &'static Budget) -> Agent {
        Agent::on("abc".into(), controller, "/nonexistent".into(), answers)
    }

    /// An agent whose answers take from a budget of their own.
    fn agent(controller: Controller) -> Agent {
        agent_on(controller, budget())
    }

    /// Waits until `ops` holds `op`.
    pub(crate) async fn recorded(ops: &std::sync::Mutex<Vec<String>>, op: &str) {
        while !ops.lock().unwrap().iter().any(|o| o == op) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn a_takeover_never_waits_for_a_call_waiting_for_its_answer_s_share() {
        let answers = budget();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let (controller, ops, _dir) = fake_controller("takeover", Some(barrier.clone())).await;
        let agent = Arc::new(agent_on(controller, answers));
        let calling = {
            let agent = agent.clone();
            tokio::spawn(async move { call(&agent, "click", json!({}), || true).await })
        };
        // While the agent works on the call, others' answers come to hold all but less than its share; then it answers,
        // and its answer waits for memory, holding the link.
        tokio::time::timeout(Duration::from_secs(10), recorded(&ops, "tools/call")).await.expect("the call was asked");
        let others = answers.take(ANSWERS_KIB - 1).unwrap();
        barrier.wait().await;
        // A share that waits holds nothing, so nothing shows when the answer starts waiting: it is given the time.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!calling.is_finished(), "the answer waits for its share");
        let start = std::time::Instant::now();
        assert!(matches!(agent.quiesce().await, Ok(Quiescence::Terminated)), "the call left its link lost");
        assert!(start.elapsed() < QUIESCENCE, "the takeover never waited for the call to time out");
        assert!(matches!(calling.await.unwrap(), Err(CallError::Blocked)));
        assert!(ops.lock().unwrap().iter().any(|o| o == "agent_terminate"));
        drop(others);
    }

    #[tokio::test]
    async fn a_short_answer_never_waits_behind_others_large_ones() {
        let answers = budget();
        let (controller, _ops, _dir) = fake_controller("short", Some(Arc::new(tokio::sync::Barrier::new(1)))).await;
        let agent = agent_on(controller, answers);
        // Others' answers hold so much that no answer of the longest kind could start, and one waits to.
        let others = answers.take(ANSWERS_KIB - kib(MAX_ANSWER) + 1).unwrap();
        let gate = Gate::new(answers);
        let mut large = Box::pin(gate.share(MAX_ANSWER));
        let polled = std::future::poll_fn(|cx| std::task::Poll::Ready(large.as_mut().poll(cx).is_pending()));
        assert!(polled.await, "the longest share waits");
        assert_eq!(answers.free(), kib(MAX_ANSWER) - 1, "and holds nothing while it waits");
        let answered = tokio::time::timeout(Duration::from_secs(10), call(&agent, "click", json!({}), || true)).await;
        assert_eq!(answered.expect("answered without waiting").unwrap(), json!({"content": []}));
        // A line longer than the buffer still takes the longest share: it waits.
        let reply = format!("{}\n", json!({"jsonrpc": "2.0", "id": 1, "result": {"text": "a".repeat(100)}}));
        let mut link = Link { stream: BufReader::with_capacity(64, peer(reply)), next_id: 1 };
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call"});
        let waiting = tokio::time::timeout(Duration::from_millis(300), exchange(&mut link, &gate, request, 1)).await;
        assert!(waiting.is_err(), "it waits for the longest share");
        // Memory given back wakes the share that waits for it.
        drop(others);
        let share = tokio::time::timeout(Duration::from_secs(10), large).await.expect("taken once given back");
        assert_eq!(share.map(|share| share.kib()), Some(kib(MAX_ANSWER)));
    }

    #[tokio::test]
    async fn two_sessions_calls_run_at_once_while_neither_waits_for_memory() {
        // Each agent answers only once both are working on their call.
        let (answers, barrier) = (budget(), Arc::new(tokio::sync::Barrier::new(2)));
        let (one, _ops, _one_dir) = fake_controller("concurrent-one", Some(barrier.clone())).await;
        let (two, _ops, _two_dir) = fake_controller("concurrent-two", Some(barrier)).await;
        let (one, two) = (agent_on(one, answers), agent_on(two, answers));
        let both = futures_util::future::join(
            call(&one, "click", json!({}), || true),
            call(&two, "click", json!({}), || true),
        );
        let (one, two) = tokio::time::timeout(Duration::from_secs(10), both).await.expect("both ran at once");
        assert_eq!((one.unwrap(), two.unwrap()), (json!({"content": []}), json!({"content": []})));
        assert_eq!(answers.free(), ANSWERS_KIB, "nothing held once their results are let go");
    }

    /// One call as a caller makes it: a place, then the call.
    async fn call(
        agent: &Agent,
        tool: &str,
        arguments: Value,
        still: impl FnOnce() -> bool,
    ) -> Result<Value, CallError> {
        let Some(mut turn) = agent.place() else { return Err(CallError::Busy) };
        agent.call(&mut turn, tool, arguments, still).await
    }

    /// A controller on a Unix socket that records each op; its first agent closes the stream on a tool call, every later
    /// one answers it. With `barrier`, every agent answers each tool call, recorded as `tools/call`, once `barrier` lets
    /// it. Its socket lives in the directory returned with it, removed when that is dropped.
    pub(crate) async fn fake_controller(
        name: &str,
        barrier: Option<Arc<tokio::sync::Barrier>>,
    ) -> (Controller, std::sync::Arc<std::sync::Mutex<Vec<String>>>, crate::private::TestDir) {
        let dir = crate::private::test_dir(&format!("agent-{name}"));
        let ops = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = ops.clone();
        let mut attached = 0;
        let controller = crate::controller::fake(dir.join("controller.sock"), move |request, mut stream| {
            let op = request["op"].as_str().unwrap().to_owned();
            recorded.lock().unwrap().push(op.clone());
            let attach = op == "agent_attach";
            attached += usize::from(attach);
            let answers = attached > 1 || barrier.is_some();
            let (barrier, recorded) = (barrier.clone(), recorded.clone());
            async move {
                stream.get_mut().write_all(b"{\"ok\":true,\"exited\":true}\n").await.unwrap();
                if !attach {
                    return;
                }
                tokio::spawn(async move {
                    let mut line = String::new();
                    while {
                        line.clear();
                        stream.read_line(&mut line).await.unwrap()
                    } > 0
                    {
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let Some(id) = request.get("id") else { continue };
                        if request["method"] == "tools/call" && !answers {
                            return;
                        }
                        if let Some(barrier) = barrier.as_ref().filter(|_| request["method"] == "tools/call") {
                            recorded.lock().unwrap().push("tools/call".into());
                            barrier.wait().await;
                        }
                        let reply = json!({"jsonrpc": "2.0", "id": id, "result": {"content": []}});
                        stream.get_mut().write_all(format!("{reply}\n").as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (controller, ops, dir)
    }

    /// A controller whose agent answers every tool call with what `answer` makes of its arguments, which it records.
    async fn driver(
        name: &str,
        answer: fn(&Value) -> Value,
    ) -> (Controller, Arc<std::sync::Mutex<Vec<Value>>>, crate::private::TestDir) {
        let dir = crate::private::test_dir(&format!("agent-{name}"));
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let controller = crate::controller::fake(dir.join("controller.sock"), move |request, mut stream| {
            let (attach, recorded) = (request["op"] == "agent_attach", recorded.clone());
            async move {
                stream.get_mut().write_all(b"{\"ok\":true}\n").await.unwrap();
                if !attach {
                    return;
                }
                tokio::spawn(async move {
                    let mut line = String::new();
                    while {
                        line.clear();
                        stream.read_line(&mut line).await.unwrap()
                    } > 0
                    {
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let Some(id) = request.get("id") else { continue };
                        let arguments = &request["params"]["arguments"];
                        let result = if request["method"] == "tools/call" { answer(arguments) } else { json!({}) };
                        recorded.lock().unwrap().push(arguments.clone());
                        let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
                        stream.get_mut().write_all(format!("{reply}\n").as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (controller, calls, dir)
    }

    #[tokio::test]
    async fn text_the_driver_cannot_type_reaches_it_empty_and_the_display_types_it_as_the_driver_would_answer() {
        let (controller, calls, dir) = driver("unicode", |arguments| {
            crate::assist::tests::typed_answer(arguments["text"].as_str().map_or(0, |t| t.chars().count()))
        })
        .await;
        let lines = crate::inject::fake(dir.join("display.sock"), |_| "ok".to_owned());
        let agent = Agent::on("abc".into(), controller, dir.join("display.sock"), budget());
        let typed = call(&agent, "type_text", json!({"pid": 7, "x": 10, "y": 20, "text": "é—中"}), || true).await;
        assert_eq!(typed.unwrap(), crate::assist::tests::typed_answer(3));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.last().unwrap(), &json!({"pid": 7, "x": 10, "y": 20, "text": ""}), "targeted by the driver");
        assert_eq!(*lines.lock().unwrap(), ["u root:7 c3a9e28094e4b8ad"]);
    }

    #[tokio::test]
    async fn text_beyond_ascii_longer_than_the_display_types_at_once_is_refused_before_the_driver_is_asked() {
        let (controller, calls, dir) = driver("too-long", |_| crate::assist::tests::typed_answer(0)).await;
        let lines = crate::inject::fake(dir.join("display.sock"), |_| "ok".to_owned());
        let agent = Agent::on("abc".into(), controller, dir.join("display.sock"), budget());
        let text = "é".repeat(600);
        let refused = call(&agent, "type_text", json!({"pid": 7, "text": text}), || true).await.unwrap();
        assert_eq!(refused["isError"], true);
        assert_eq!(
            refused["structuredContent"],
            json!({"code": "text_too_long", "max_bytes": display_protocol::MAX_TEXT})
        );
        assert!(calls.lock().unwrap().is_empty() && lines.lock().unwrap().is_empty(), "nothing asked of either");
        assert!(matches!(*agent.link.lock().await, State::Fresh), "the link as it was");
        // At the bound, it goes.
        let text = "é".repeat(display_protocol::MAX_TEXT / 2);
        let typed = call(&agent, "type_text", json!({"pid": 7, "text": text}), || true).await.unwrap();
        assert_eq!(typed, crate::assist::tests::typed_answer(display_protocol::MAX_TEXT / 2));
        assert_eq!(lines.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_idle_agent_is_claimed_and_one_in_use_is_left_as_it_is() {
        let fresh = || Agent::new("abc".into(), Controller::new("/nonexistent".into()), "/nonexistent".into());
        let (long, none) = (Duration::from_secs(3600), Duration::ZERO);
        let agent = fresh();
        assert!(agent.claim_if_idle(long) && agent.is_blocked(), "nothing yet: claimed, and blocked");
        // An agent with a call placed is not claimed; a call placed after the claim is refused.
        let agent = fresh();
        let turn = agent.place().unwrap();
        assert!(!agent.claim_if_idle(none) && !agent.is_blocked());
        drop(turn);
        assert!(!agent.claim_if_idle(long), "a call that ended, for a while");
        assert!(agent.claim_if_idle(none));
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Blocked)));
        // A refused call counts too: that agent is trying.
        let agent = fresh();
        agent.gate.block();
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Blocked)));
        assert!(!agent.claim_if_idle(long));
        // A link a takeover holds, or a lost one.
        let agent = fresh();
        let link = agent.link.lock().await;
        assert!(!agent.claim_if_idle(none) && !agent.is_blocked());
        drop(link);
        *agent.link.lock().await = State::Lost;
        assert!(!agent.claim_if_idle(none), "a lost link needs the agent killed: an explicit takeover's work");
        // The agent's start of the session.
        let agent = fresh();
        agent.used_now();
        assert!(!agent.claim_if_idle(long) && agent.claim_if_idle(none));
    }

    #[tokio::test]
    async fn a_call_whose_caller_left_keeps_its_agent_in_use_until_it_ends() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let (controller, ops, _dir) = fake_controller("detached", Some(barrier.clone())).await;
        let agent = Arc::new(agent(controller));
        let calling = {
            let agent = agent.clone();
            tokio::spawn(async move { call(&agent, "click", json!({}), || true).await })
        };
        tokio::time::timeout(Duration::from_secs(10), recorded(&ops, "tools/call")).await.expect("the call was asked");
        calling.abort();
        let _ = calling.await;
        assert_eq!(agent.waiting.load(Ordering::SeqCst), 0, "its caller gave its place back");
        assert_eq!(agent.activity.lock().unwrap().running, 1, "the call runs on");
        assert!(!agent.claim_if_idle(Duration::ZERO));
        // Its caller's leaving is forgotten: only the call's own end may count from here.
        agent.activity.lock().unwrap().last = None;
        barrier.wait().await;
        let ended = async {
            while agent.activity.lock().unwrap().running > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), ended).await.expect("the call ended");
        assert!(!agent.claim_if_idle(Duration::from_secs(3600)), "its end is recorded");
        assert!(agent.claim_if_idle(Duration::ZERO));
    }

    #[tokio::test]
    async fn a_stopped_agent_stays_blocked_whatever_resumes_after() {
        let agent = agent(Controller::new("/nonexistent".into()));
        agent.detach();
        agent.resume(false).await.unwrap();
        agent.gate.block();
        agent.resume(false).await.unwrap();
        assert!(agent.is_blocked() && !agent.claim_if_idle(Duration::ZERO));
        // A takeover's block is lifted by its give-back.
        let agent = super::tests::agent(Controller::new("/nonexistent".into()));
        agent.gate.block();
        agent.resume(false).await.unwrap();
        assert!(!agent.is_blocked());
    }

    #[tokio::test]
    async fn a_lost_link_restarts_the_agent_before_attaching_again() {
        let (controller, ops, _dir) = fake_controller("lost", None).await;
        let agent = agent(controller);
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Failed(_))));
        assert!(matches!(*agent.link.lock().await, State::Lost));
        assert_eq!(call(&agent, "click", json!({}), || true).await.unwrap(), json!({"content": []}));
        assert_eq!(*ops.lock().unwrap(), ["agent_attach", "agent_terminate", "agent_restart", "agent_attach"]);
        // An open link is used again as it is.
        call(&agent, "click", json!({}), || true).await.unwrap();
        assert_eq!(ops.lock().unwrap().len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn calls_waiting_for_the_agent_are_bounded_and_wait_a_bounded_time() {
        let agent = Arc::new(agent(Controller::new("/nonexistent".into())));
        // A call in flight holds the link.
        let in_flight = agent.link.clone().lock_owned().await;
        let mut waiting = Vec::new();
        for _ in 0..MAX_CALLS {
            let agent = agent.clone();
            waiting.push(tokio::spawn(async move { call(&agent, "click", json!({}), || true).await }));
        }
        tokio::task::yield_now().await;
        while agent.waiting.load(Ordering::SeqCst) < MAX_CALLS {
            tokio::task::yield_now().await;
        }
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Busy)), "one more is refused");
        tokio::time::advance(WAIT_DEADLINE + Duration::from_millis(1)).await;
        for call in waiting {
            assert!(matches!(call.await.unwrap(), Err(CallError::Busy)), "each gave up at the deadline");
        }
        assert_eq!(agent.waiting.load(Ordering::SeqCst), 0, "nothing stays queued");
        // A caller that leaves while it waits gives its place back.
        let leaving = {
            let agent = agent.clone();
            tokio::spawn(async move { call(&agent, "click", json!({}), || true).await })
        };
        while agent.waiting.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
        leaving.abort();
        let _ = leaving.await;
        assert_eq!(agent.waiting.load(Ordering::SeqCst), 0, "a cancelled wait keeps no place");
        drop(in_flight);
        // The link is free again: a call is admitted (and withdrawn here before it asks the agent anything).
        assert!(matches!(call(&agent, "click", json!({}), || false).await, Err(CallError::Withdrawn)));
    }

    #[tokio::test]
    async fn a_call_its_caller_may_no_longer_make_asks_nothing_of_the_agent() {
        let (controller, ops, _dir) = fake_controller("withdrawn", None).await;
        let agent = agent(controller);
        assert!(matches!(call(&agent, "click", json!({}), || false).await, Err(CallError::Withdrawn)));
        assert!(ops.lock().unwrap().is_empty(), "not even attached");
        assert!(matches!(*agent.link.lock().await, State::Fresh));
    }

    #[tokio::test]
    async fn a_result_keeps_its_call_s_place_until_its_caller_lets_it_go() {
        let (controller, _ops, _dir) = fake_controller("results", None).await;
        let agent = agent(controller);
        // The fake's first agent never answers; the restarted one does.
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Failed(_))));
        let mut held = Vec::new();
        for _ in 0..MAX_CALLS {
            let mut turn = agent.place().unwrap();
            agent.call(&mut turn, "click", json!({}), || true).await.unwrap();
            held.push(turn);
        }
        assert!(matches!(call(&agent, "click", json!({}), || true).await, Err(CallError::Busy)), "no more results");
        drop(held);
        assert_eq!(call(&agent, "click", json!({}), || true).await.unwrap(), json!({"content": []}));
    }

    #[tokio::test]
    async fn a_takeover_during_a_lost_link_s_restart_leaves_the_agent_stopped_until_the_give_back() {
        let (controller, ops, _dir) = fake_controller("blocked", None).await;
        let agent = agent(controller.clone());
        // The call was admitted, then a takeover blocked the agent while it found its link lost.
        *agent.link.lock().await = State::Lost;
        let guard = agent.link.clone().lock_owned().await;
        agent.gate.block();
        let call = Call::new("click", json!({}), agent.assist.clone());
        let (_, result) = run(guard, controller, "abc".into(), agent.gate.clone(), call).await;
        assert!(matches!(result, Err(CallError::Blocked)));
        assert_eq!(*ops.lock().unwrap(), ["agent_terminate"], "never restarted while a person holds control");
        assert!(matches!(*agent.link.lock().await, State::Lost));
        // The give-back terminates (a no-op here) before it restarts, so a fresh agent left running is no obstacle.
        agent.resume(true).await.unwrap();
        assert_eq!(*ops.lock().unwrap(), ["agent_terminate", "agent_terminate", "agent_restart"]);
        assert!(!agent.is_blocked());
    }
}
