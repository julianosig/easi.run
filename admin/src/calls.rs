//! A session's desktop calls, as an agent reconciling after a lost answer needs them: the calls running now, and a
//! receipt for each call made with a `call_id`, an id the agent chose for it, so that the same call sent again (a retry
//! after a timeout or a lost connection) is answered with how the first one went rather than made twice.
//!
//! A receipt is taken before its call waits for its turn, so two retries sent together make one call: the second finds
//! the first's. A call that never reached the agent (refused while a person holds control, too many waiting, its caller
//! gone before its turn) gives its receipt back, so the same id can be sent again; once it may have reached the agent,
//! its receipt stays for as long as the session runs ([`MAX_RECEIPTS`] at most), and ends as the call did: `unknown`
//! when the answer never said. Results are not kept: a receipt says what ran and how it ended, and the agent reads the
//! window again. Only ids, tools, times, outcomes and who called are kept (no arguments, nothing a page wrote), in the
//! admin's memory: a stop or a restart of the admin forgets them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::timeline::now_ms;

/// The most receipts one session keeps: past it, a call with a new `call_id` is refused.
pub const MAX_RECEIPTS: usize = 1000;
/// The longest `call_id`.
pub const MAX_ID: usize = 64;

/// Whether `id` can be a `call_id`: 1 to [`MAX_ID`] letters, digits, `-` or `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What makes two calls the same call: the tool, and its arguments as sent (objects' keys in any order, everything
/// else exactly: arrays in order, strings and numbers as written).
pub fn fingerprint(tool: &str, arguments: &Value) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(tool.as_bytes());
    hash.update([0]);
    // Without `preserve_order`, serde_json's objects are sorted maps: the same call writes the same bytes.
    hash.update(serde_json::to_vec(arguments).expect("JSON values encode"));
    hash.finalize().into()
}

#[derive(Default)]
pub struct Calls {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    receipts: HashMap<String, Receipt>,
    running: Vec<Running>,
    next: u64,
}

struct Receipt {
    fingerprint: [u8; 32],
    tool: String,
    by: Value,
    started: u64,
    state: State,
}

#[derive(Clone, Copy)]
enum State {
    /// Taken, its call waiting for its turn.
    Waiting,
    Running,
    Ended {
        outcome: Outcome,
        at: u64,
    },
}

/// How a call that reached the agent ended, as a receipt says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The tool's result came back.
    Ok,
    /// The tool's result came back saying it failed (`isError`).
    Error,
    /// The call failed on the way, after it may have reached the agent.
    Failed,
    /// No answer said how it went.
    Unknown,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Error => "error",
            Outcome::Failed => "failed",
            Outcome::Unknown => "unknown",
        }
    }
}

struct Running {
    seq: u64,
    tool: String,
    by: Value,
    started: u64,
    call_id: Option<String>,
}

/// Why a call with a `call_id` is not made.
pub enum Known {
    /// That id named this very call before: its receipt.
    Same(Value),
    /// That id named another call (another tool, or other arguments).
    Other,
    /// The session holds [`MAX_RECEIPTS`] already.
    Full,
}

/// A receipt taken for a call that has not reached the agent yet: dropped before [`Calls::started`] took it on, it is
/// given back.
pub struct Reservation {
    calls: Arc<Calls>,
    id: String,
    taken: bool,
}

impl Reservation {
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.taken {
            let mut inner = self.calls.inner.lock().unwrap();
            if inner.receipts.get(&self.id).is_some_and(|r| matches!(r.state, State::Waiting)) {
                inner.receipts.remove(&self.id);
            }
        }
    }
}

/// A running call's place in [`Calls`]: ended as its call ends, or as unknown when dropped first.
pub struct Started {
    calls: Arc<Calls>,
    seq: u64,
    call_id: Option<String>,
    ended: bool,
}

impl Started {
    pub fn call_id(&self) -> Option<&str> {
        self.call_id.as_deref()
    }

    pub fn end(mut self, outcome: Outcome) {
        self.finish(outcome);
    }

    fn finish(&mut self, outcome: Outcome) {
        self.ended = true;
        let mut inner = self.calls.inner.lock().unwrap();
        inner.running.retain(|r| r.seq != self.seq);
        if let Some(receipt) = self.call_id.as_ref().and_then(|id| inner.receipts.get_mut(id)) {
            receipt.state = State::Ended { outcome, at: now_ms() };
        }
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if !self.ended {
            self.finish(Outcome::Unknown);
        }
    }
}

impl Calls {
    /// Takes the receipt `id` for a call of `tool` with `fingerprint` by `by`, or says why the call is not made.
    pub fn reserve(
        self: &Arc<Self>,
        id: &str,
        fingerprint: [u8; 32],
        tool: &str,
        by: &Value,
    ) -> Result<Reservation, Known> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(receipt) = inner.receipts.get(id) {
            return Err(if receipt.fingerprint == fingerprint { Known::Same(said(id, receipt)) } else { Known::Other });
        }
        if inner.receipts.len() >= MAX_RECEIPTS {
            return Err(Known::Full);
        }
        let receipt =
            Receipt { fingerprint, tool: tool.to_owned(), by: by.clone(), started: now_ms(), state: State::Waiting };
        inner.receipts.insert(id.to_owned(), receipt);
        Ok(Reservation { calls: self.clone(), id: id.to_owned(), taken: false })
    }

    /// A call of `tool` by `by` reaching the agent now, with its receipt if it has one.
    pub fn started(self: &Arc<Self>, tool: &str, by: &Value, reservation: Option<Reservation>) -> Started {
        let mut inner = self.inner.lock().unwrap();
        let seq = inner.next;
        inner.next += 1;
        let started = now_ms();
        let call_id = reservation.map(|mut r| {
            r.taken = true;
            std::mem::take(&mut r.id)
        });
        if let Some(receipt) = call_id.as_ref().and_then(|id| inner.receipts.get_mut(id)) {
            (receipt.state, receipt.started) = (State::Running, started);
        }
        let running = Running { seq, tool: tool.to_owned(), by: by.clone(), started, call_id: call_id.clone() };
        inner.running.push(running);
        Started { calls: self.clone(), seq, call_id, ended: false }
    }

    /// The receipt `id`, if this session has it.
    pub fn receipt(&self, id: &str) -> Option<Value> {
        let inner = self.inner.lock().unwrap();
        inner.receipts.get(id).map(|receipt| said(id, receipt))
    }

    /// The calls running now, oldest first.
    pub fn running(&self) -> Vec<Value> {
        let inner = self.inner.lock().unwrap();
        let said = |r: &Running| {
            let mut call = json!({"tool": r.tool, "started": r.started, "call_id": r.call_id});
            merge(&mut call, &r.by);
            call
        };
        inner.running.iter().map(said).collect()
    }
}

/// A receipt as an agent reads it: its id, tool, state (`waiting`, `running`, or how it ended), times and who called.
fn said(id: &str, receipt: &Receipt) -> Value {
    let (state, ended) = match receipt.state {
        State::Waiting => ("waiting", None),
        State::Running => ("running", None),
        State::Ended { outcome, at } => (outcome.as_str(), Some(at)),
    };
    let mut said =
        json!({"call_id": id, "tool": receipt.tool, "state": state, "started": receipt.started, "ended": ended});
    merge(&mut said, &receipt.by);
    said
}

/// `by`'s fields (who called, see `service::caller`) added to `into`.
fn merge(into: &mut Value, by: &Value) {
    if let (Some(into), Some(by)) = (into.as_object_mut(), by.as_object()) {
        into.extend(by.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by() -> Value {
        json!({"via": "token", "token": "ci"})
    }

    #[test]
    fn ids_are_short_plain_names() {
        assert!(valid_id("a") && valid_id(&"x".repeat(MAX_ID)) && valid_id("submit-order_2"));
        assert!(!valid_id("") && !valid_id(&"x".repeat(MAX_ID + 1)) && !valid_id("a b") && !valid_id("é"));
    }

    #[test]
    fn the_same_call_is_its_arguments_whatever_their_keys_order() {
        let a = fingerprint("click", &serde_json::from_str(r#"{"x": 1, "y": [1, 2]}"#).unwrap());
        assert_eq!(a, fingerprint("click", &serde_json::from_str(r#"{"y": [1, 2], "x": 1}"#).unwrap()));
        assert_ne!(a, fingerprint("click", &json!({"x": 1, "y": [2, 1]})), "arrays in order");
        assert_ne!(a, fingerprint("click", &json!({"x": 1.0, "y": [1, 2]})), "numbers as written");
        assert_ne!(a, fingerprint("type_text", &json!({"x": 1, "y": [1, 2]})), "the tool");
    }

    #[test]
    fn a_receipt_names_one_call_until_the_session_ends_and_one_never_made_is_given_back() {
        let calls = Arc::new(Calls::default());
        let print = fingerprint("click", &json!({"x": 1}));
        let first = calls.reserve("buy", print, "click", &by()).ok().unwrap();
        // Sent again while the first waits: not made, and it says so.
        let Err(Known::Same(again)) = calls.reserve("buy", print, "click", &by()) else { panic!("known") };
        assert_eq!((again["state"].as_str(), again["token"].as_str()), (Some("waiting"), Some("ci")));
        assert!(matches!(
            calls.reserve("buy", fingerprint("click", &json!({"x": 2})), "click", &by()),
            Err(Known::Other)
        ));
        // The first never reached the agent: given back, the id can be used again.
        drop(first);
        assert!(calls.receipt("buy").is_none());
        let first = calls.reserve("buy", print, "click", &by()).ok().unwrap();
        let running = calls.started("click", &by(), Some(first));
        assert_eq!(calls.receipt("buy").unwrap()["state"], "running");
        assert_eq!(calls.running()[0]["call_id"], "buy");
        running.end(Outcome::Ok);
        assert!(calls.running().is_empty());
        let ended = calls.receipt("buy").unwrap();
        assert_eq!(ended["state"], "ok");
        assert!(ended["ended"].as_u64().unwrap() >= ended["started"].as_u64().unwrap());
        // An answer that never came: unknown.
        let lost = calls.reserve("pay", fingerprint("click", &json!({})), "click", &by()).ok().unwrap();
        drop(calls.started("click", &by(), Some(lost)));
        assert_eq!(calls.receipt("pay").unwrap()["state"], "unknown");
        // A call without an id is listed while it runs, and leaves no receipt.
        let plain = calls.started("scroll", &by(), None);
        assert_eq!(calls.running()[0]["call_id"], Value::Null);
        plain.end(Outcome::Error);
        assert!(calls.running().is_empty());
    }

    #[test]
    fn a_session_keeps_a_bounded_number_of_receipts() {
        let calls = Arc::new(Calls::default());
        for n in 0..MAX_RECEIPTS {
            let taken = calls.reserve(&n.to_string(), [0; 32], "click", &by()).ok().unwrap();
            calls.started("click", &by(), Some(taken)).end(Outcome::Ok);
        }
        assert!(matches!(calls.reserve("one-more", [0; 32], "click", &by()), Err(Known::Full)));
        assert!(matches!(calls.reserve("0", [0; 32], "click", &by()), Err(Known::Same(_))), "known ones still answer");
    }
}
