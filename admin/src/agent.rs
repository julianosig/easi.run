//! The automation agent of one session: the Cua Driver's MCP server, reached through the controller's `agent_attach`.
//!
//! Calls run one at a time. Taking control first blocks new calls, then waits a bounded time for the in-flight call's
//! reply; without it, the controller kills the agent container and must prove it stopped. Only then is control
//! granted; if no proof comes, the whole session is stopped. Closing the MCP stream alone is never taken as proof.
//! A link that fails is never attached again as it is: the controller gives an agent container one exec, which may
//! still run, so the next call terminates and restarts the agent first.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, OwnedMutexGuard};

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
/// Calls that may wait for the one in flight, per session, and how long each may wait: more, or longer, are refused
/// as busy, so callers cannot pile work up behind a slow call.
pub const MAX_WAITING: usize = 4;
pub const WAIT_DEADLINE: Duration = Duration::from_secs(30);
const MAX_LINE: usize = 32 * 1024 * 1024;

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
    blocked: Arc<AtomicBool>,
    /// Calls waiting for the link now (see [`MAX_WAITING`]).
    waiting: std::sync::atomic::AtomicUsize,
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
    pub fn new(session: String, controller: Controller) -> Agent {
        Agent {
            session,
            controller,
            link: Arc::new(Mutex::new(State::Fresh)),
            blocked: Arc::new(AtomicBool::new(false)),
            waiting: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::SeqCst)
    }

    /// Runs one tool and returns its MCP result. Waiting for the previous call is the caller's (a caller that leaves
    /// before its turn asks nothing of the agent); once admitted, the exchange runs as its own task holding the link,
    /// so a caller that disconnects never cuts it half way. `still` says whether the caller may still call once it is
    /// admitted (a token revoked meanwhile may not).
    pub async fn call(&self, tool: &str, arguments: Value, still: impl FnOnce() -> bool) -> Result<Value, CallError> {
        if self.is_blocked() {
            return Err(CallError::Blocked);
        }
        let guard = {
            let Some(_place) = Waiting::take(&self.waiting) else { return Err(CallError::Busy) };
            // Past the deadline the wait is dropped: nothing stays queued. A caller that leaves while it waits gives its
            // place back too (`Waiting` is dropped either way).
            tokio::time::timeout(WAIT_DEADLINE, self.link.clone().lock_owned()).await.map_err(|_| CallError::Busy)?
        };
        // A takeover may have started while this call waited for the previous one.
        if self.is_blocked() {
            return Err(CallError::Blocked);
        }
        if !still() {
            return Err(CallError::Withdrawn);
        }
        let (controller, session, blocked) = (self.controller.clone(), self.session.clone(), self.blocked.clone());
        let tool = tool.to_owned();
        tokio::spawn(run(guard, controller, session, blocked, tool, arguments))
            .await
            .unwrap_or_else(|_| Err(CallError::Failed("the agent call failed".into())))
    }

    /// Blocks new calls and proves the agent is quiescent: the in-flight call answered in time, or the agent's
    /// container was killed and proven stopped. An error means neither proof came and the session must be stopped.
    pub async fn quiesce(&self) -> Result<Quiescence, String> {
        self.blocked.store(true, Ordering::SeqCst);
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

    /// Blocks new calls and closes an idle link, so a session being stopped holds no controller connection (each link
    /// holds one); a call in flight drops its link itself once the stopped agent's stream ends.
    pub fn detach(&self) {
        self.blocked.store(true, Ordering::SeqCst);
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
        self.blocked.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// One of the [`MAX_WAITING`] places to wait for the link, given back when dropped.
struct Waiting<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> Waiting<'a> {
    fn take(waiting: &'a std::sync::atomic::AtomicUsize) -> Option<Waiting<'a>> {
        let taken = waiting.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < MAX_WAITING).then_some(n + 1));
        // Built only once taken: a refused one must give nothing back.
        taken.is_ok().then(|| Waiting(waiting))
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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

/// One admitted call, holding the link: attaches (restarting a lost agent first), asks, and puts the link back only once
/// the agent answered, so a call that fails, or is dropped, leaves it lost.
async fn run(
    mut guard: OwnedMutexGuard<State>,
    controller: Controller,
    session: String,
    blocked: Arc<AtomicBool>,
    tool: String,
    arguments: Value,
) -> Result<Value, CallError> {
    let mut link = match std::mem::replace(&mut *guard, State::Lost) {
        State::Open(link) => link,
        State::Fresh => attach(&controller, &session).await.map_err(CallError::Failed)?,
        State::Lost => {
            tracing::warn!(%session, "the agent's link was lost; restarting the agent");
            controller.agent_terminate(&session).await.map_err(CallError::Failed)?;
            // A takeover that started meanwhile wants the agent stopped: no fresh one while a person holds control.
            // The link stays lost, so the give-back's restart (or the next call) starts it.
            if blocked.load(Ordering::SeqCst) {
                return Err(CallError::Blocked);
            }
            controller.agent_restart(&session).await.map_err(CallError::Failed)?;
            attach(&controller, &session).await.map_err(CallError::Failed)?
        }
    };
    // A takeover may have started while this call attached; nothing was asked of the agent yet.
    if blocked.load(Ordering::SeqCst) {
        *guard = State::Open(link);
        return Err(CallError::Blocked);
    }
    let id = link.next_id;
    link.next_id += 1;
    let deadline = deadline(&tool, &arguments);
    let request =
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
    match tokio::time::timeout(deadline, exchange(&mut link, request, id)).await {
        Ok(Ok(mut response)) => {
            *guard = State::Open(link);
            // Moved out, not copied: a result may be tens of MiB.
            match response.as_object_mut().and_then(|m| m.remove("result")) {
                Some(result) => Ok(result),
                None => Err(CallError::Failed(response.get("error").map_or("no result".into(), Value::to_string))),
            }
        }
        Ok(Err(e)) => Err(CallError::Failed(e)),
        Err(_) => Err(CallError::Failed("the agent did not answer in time".into())),
    }
}

async fn attach(controller: &Controller, session: &str) -> Result<Link, String> {
    let stream = controller.agent_attach(session).await?;
    let mut link = Link { stream, next_id: 1 };
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "easi-run-admin", "version": env!("CARGO_PKG_VERSION")}}});
    tokio::time::timeout(ATTACH_DEADLINE, exchange(&mut link, init, 0))
        .await
        .map_err(|_| "the agent did not initialize in time".to_owned())??;
    send(&mut link, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await?;
    Ok(link)
}

async fn send(link: &mut Link, message: &Value) -> Result<(), String> {
    let mut line = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    line.push(b'\n');
    link.stream.get_mut().write_all(&line).await.map_err(|e| format!("agent: {e}"))
}

/// Sends one request and reads lines until its response, skipping notifications and other ids.
async fn exchange(link: &mut Link, request: Value, id: u64) -> Result<Value, String> {
    send(link, &request).await?;
    loop {
        let mut line = String::new();
        let n =
            (&mut link.stream).take(MAX_LINE as u64).read_line(&mut line).await.map_err(|e| format!("agent: {e}"))?;
        if n == 0 {
            return Err("the agent's stream closed".into());
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        if message.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(message);
        }
    }
}

#[cfg(test)]
mod tests {
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
    async fn detaching_closes_an_idle_link_and_blocks_new_calls() {
        let agent = Agent::new("abc".into(), Controller::new("/nonexistent".into()));
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        *agent.link.lock().await = State::Open(Link { stream: BufReader::new(ours), next_id: 1 });
        agent.detach();
        assert!(matches!(*agent.link.lock().await, State::Lost));
        let mut rest = Vec::new();
        assert_eq!(theirs.read_to_end(&mut rest).await.unwrap(), 0, "the controller sees the link close");
        assert!(matches!(agent.call("click", json!({}), || true).await, Err(CallError::Blocked)));
    }

    /// A controller on a Unix socket that records each op; its first agent closes the stream on a tool call, every later
    /// one answers it.
    async fn fake_controller(name: &str) -> (Controller, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let path = std::env::temp_dir().join(format!("agent-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let ops = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = ops.clone();
        tokio::spawn(async move {
            let mut attached = 0;
            while let Ok((stream, _)) = listener.accept().await {
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let op = serde_json::from_str::<Value>(&line).unwrap()["op"].as_str().unwrap().to_owned();
                recorded.lock().unwrap().push(op.clone());
                stream.get_mut().write_all(b"{\"ok\":true,\"exited\":true}\n").await.unwrap();
                if op != "agent_attach" {
                    continue;
                }
                attached += 1;
                let answers = attached > 1;
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
                        let reply = json!({"jsonrpc": "2.0", "id": id, "result": {"content": []}});
                        stream.get_mut().write_all(format!("{reply}\n").as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (Controller::new(path), ops)
    }

    #[tokio::test]
    async fn a_lost_link_restarts_the_agent_before_attaching_again() {
        let (controller, ops) = fake_controller("lost").await;
        let agent = Agent::new("abc".into(), controller);
        assert!(matches!(agent.call("click", json!({}), || true).await, Err(CallError::Failed(_))));
        assert!(matches!(*agent.link.lock().await, State::Lost));
        assert_eq!(agent.call("click", json!({}), || true).await.unwrap(), json!({"content": []}));
        assert_eq!(*ops.lock().unwrap(), ["agent_attach", "agent_terminate", "agent_restart", "agent_attach"]);
        // An open link is used again as it is.
        agent.call("click", json!({}), || true).await.unwrap();
        assert_eq!(ops.lock().unwrap().len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn calls_waiting_for_the_agent_are_bounded_and_wait_a_bounded_time() {
        let agent = Arc::new(Agent::new("abc".into(), Controller::new("/nonexistent".into())));
        // A call in flight holds the link.
        let in_flight = agent.link.clone().lock_owned().await;
        let mut waiting = Vec::new();
        for _ in 0..MAX_WAITING {
            let agent = agent.clone();
            waiting.push(tokio::spawn(async move { agent.call("click", json!({}), || true).await }));
        }
        tokio::task::yield_now().await;
        while agent.waiting.load(Ordering::SeqCst) < MAX_WAITING {
            tokio::task::yield_now().await;
        }
        assert!(matches!(agent.call("click", json!({}), || true).await, Err(CallError::Busy)), "one more is refused");
        tokio::time::advance(WAIT_DEADLINE + Duration::from_millis(1)).await;
        for call in waiting {
            assert!(matches!(call.await.unwrap(), Err(CallError::Busy)), "each gave up at the deadline");
        }
        assert_eq!(agent.waiting.load(Ordering::SeqCst), 0, "nothing stays queued");
        // A caller that leaves while it waits gives its place back.
        let leaving = {
            let agent = agent.clone();
            tokio::spawn(async move { agent.call("click", json!({}), || true).await })
        };
        while agent.waiting.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
        leaving.abort();
        let _ = leaving.await;
        assert_eq!(agent.waiting.load(Ordering::SeqCst), 0, "a cancelled wait keeps no place");
        drop(in_flight);
        // The link is free again: a call is admitted (and withdrawn here before it asks the agent anything).
        assert!(matches!(agent.call("click", json!({}), || false).await, Err(CallError::Withdrawn)));
    }

    #[tokio::test]
    async fn a_call_its_caller_may_no_longer_make_asks_nothing_of_the_agent() {
        let (controller, ops) = fake_controller("withdrawn").await;
        let agent = Agent::new("abc".into(), controller);
        assert!(matches!(agent.call("click", json!({}), || false).await, Err(CallError::Withdrawn)));
        assert!(ops.lock().unwrap().is_empty(), "not even attached");
        assert!(matches!(*agent.link.lock().await, State::Fresh));
    }

    #[tokio::test]
    async fn a_takeover_during_a_lost_link_s_restart_leaves_the_agent_stopped_until_the_give_back() {
        let (controller, ops) = fake_controller("blocked").await;
        let agent = Agent::new("abc".into(), controller.clone());
        // The call was admitted, then a takeover blocked the agent while it found its link lost.
        *agent.link.lock().await = State::Lost;
        let guard = agent.link.clone().lock_owned().await;
        agent.blocked.store(true, Ordering::SeqCst);
        let result = run(guard, controller, "abc".into(), agent.blocked.clone(), "click".into(), json!({})).await;
        assert!(matches!(result, Err(CallError::Blocked)));
        assert_eq!(*ops.lock().unwrap(), ["agent_terminate"], "never restarted while a person holds control");
        assert!(matches!(*agent.link.lock().await, State::Lost));
        // The give-back terminates (a no-op here) before it restarts, so a fresh agent left running is no obstacle.
        agent.resume(true).await.unwrap();
        assert_eq!(*ops.lock().unwrap(), ["agent_terminate", "agent_terminate", "agent_restart"]);
        assert!(!agent.is_blocked());
    }
}
