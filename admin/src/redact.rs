//! The redaction contract after a person takes control.
//!
//! Every string the person typed or pasted is kept (bounded, in memory only) and removed from every later agent
//! observation; after the first takeover no screenshot reaches the agent's caller; and accessibility values of
//! password fields are never passed on. What is typed is followed as a run with a cursor through the edits the relay
//! can follow exactly. After one it cannot (a shortcut, a selection, an edit next to a character outside ASCII), the
//! cursor is no longer followed and the redaction errs on the side of hiding more: every piece of [`MIN_LENGTH`]
//! characters of what was typed is removed wherever it appears. A value the page transformed (masked, split, encoded)
//! cannot be recognized and is the stated residual. When the set is full, nothing of an agent's observations is passed
//! on any more.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use serde_json::Value;

/// What the set holds: far more than a person types or pastes (a paste is at most 64 KiB), and a matcher the agent's
/// next call builds in tens of milliseconds.
pub const MAX_BYTES: usize = 256 * 1024;
/// Shorter strings are not searched for: removing every occurrence of one or two characters would erase the
/// observation itself. They are the second stated residual.
pub const MIN_LENGTH: usize = 4;
/// The bytes a typed run holds. Each deletion keeps every piece of the run (see [`Redactions::edit`]), so this bounds
/// what one keystroke costs; a run that grows past it is kept with every piece of it, and what is typed after goes on
/// as a run whose cursor is lost (see [`Redactions::lose`]). A longer paste is kept whole, not followed (see
/// [`Redactions::pasted`]).
const MAX_RUN: usize = 256;
const MARK: &str = "[redacted]";

#[derive(Default, Debug, Clone)]
pub struct Redactions {
    /// Shared with the [`Scrub`]s taken of it, so that one takes them out of the session's lock without a copy.
    entries: Arc<HashSet<String>>,
    /// Finds every entry, overlapping matches included, in one scan of the original text. Built only when an agent
    /// call needs it, outside the session's lock (see [`Redactions::install`]): none while entries were recorded since.
    matcher: Option<Arc<Matcher>>,
    bytes: usize,
    /// The run of characters typed since the last field boundary.
    typing: Run,
    /// Set by an edit the run cannot follow, until the next field boundary: what is typed then is only collected, as
    /// the piece typed since the last key that was not a character (see [`Redactions::lose`]).
    lost: bool,
    /// Set once the set could not hold what the person typed: from then on nothing of an observation is passed on.
    pub saturated: bool,
    /// Set by the first takeover; from then on screenshots never leave.
    pub screenshots_blocked: bool,
}

impl Redactions {
    /// Records a pasted text or a finished typed run.
    pub fn keep(&mut self, text: &str) {
        self.add(&[text]);
    }

    /// Records `text`, and every piece of it [`MIN_LENGTH`] characters long, so that whatever part of it a field kept
    /// after edits that were not followed stays hidden (and the same pieces anywhere else too).
    fn keep_pieces(&mut self, text: &str) {
        self.add(&[text]);
        self.add_pieces(text);
    }

    /// Records every piece of `text` [`MIN_LENGTH`] characters long, and of `text` without its line breaks, as a
    /// single-line field holds it.
    fn add_pieces(&mut self, text: &str) {
        let single = text.replace(['\n', '\r'], "");
        for text in if single.len() < text.len() { vec![text, single.as_str()] } else { vec![text] } {
            let text = text.trim_matches(|c: char| c == '\n' || c == '\r');
            let starts: Vec<usize> = text.char_indices().map(|(at, _)| at).chain([text.len()]).collect();
            let pieces: Vec<&str> = starts.windows(MIN_LENGTH + 1).map(|w| &text[w[0]..w[MIN_LENGTH]]).collect();
            self.add(&pieces);
        }
    }

    /// Adds `texts` to the set, in time bounded by their length, up to [`MAX_BYTES`]: each as it is and, where it
    /// differs, as a JSON string holds it (a quote, a backslash or a control character escaped), the form a driver
    /// result's text carries a value in (`run_script`'s returned value, an error's message).
    fn add(&mut self, texts: &[&str]) {
        for text in texts {
            let text = text.trim_matches(|c: char| c == '\n' || c == '\r');
            if text.chars().count() < MIN_LENGTH {
                continue;
            }
            let quoted = serde_json::to_string(text).expect("a string is JSON");
            let escaped = &quoted[1..quoted.len() - 1];
            let forms = [text, escaped];
            for form in &forms[..if escaped == text { 1 } else { 2 }] {
                if self.entries.contains(*form) {
                    continue;
                }
                if self.bytes + form.len() > MAX_BYTES {
                    self.saturated = true;
                    return;
                }
                Arc::make_mut(&mut self.entries).insert((*form).to_owned());
                self.bytes += form.len();
                self.matcher = None;
            }
        }
    }

    /// What applies the contract to an agent's observations as the set stands, cheap to take under the session's
    /// lock: the entries and their matcher, if it is built, else [`Scrub::build`] builds it outside that lock.
    pub fn scrub(&self) -> Scrub {
        Scrub {
            entries: self.entries.clone(),
            matcher: self.matcher.clone(),
            screenshots_blocked: self.screenshots_blocked,
            hidden: self.saturated,
        }
    }

    /// Keeps the matcher `scrub` built for the next agent call, unless an entry was recorded since it was taken.
    /// Recording only collects, so a person's keystrokes never pay for an automaton over the whole set; the agent's
    /// calls build it, once per change.
    pub fn install(&mut self, scrub: &Scrub) {
        // Recording into entries a scrub still shares copies them first (`Arc::make_mut`): the same ones are unchanged.
        if self.matcher.is_none() && Arc::ptr_eq(&self.entries, &scrub.entries) {
            self.matcher = scrub.matcher.clone();
        }
    }

    /// Text the person pasted at the cursor: an entry of its own, and part of the run, which goes on around it, so that
    /// the field's value as the run follows it stays hidden whole, and so does the run as it stood, which an undo
    /// brings back (see [`Redactions::undoable`]). A paste with a line break, or longer than a run holds, is kept as it
    /// is, as a multi-line field holds it (each line break a line feed), without its line breaks (as a single-line field
    /// holds it), and line by line, and the run is lost: a short one goes on as the piece collected.
    pub fn pasted(&mut self, text: &str) {
        // Once the set is full, nothing of an observation is passed on: there is nothing more to record.
        if self.saturated {
            return;
        }
        self.undoable();
        let multi = text.contains(['\n', '\r']);
        if multi || text.len() > MAX_RUN {
            let (single, lines) = (text.replace(['\n', '\r'], ""), text.replace("\r\n", "\n").replace('\r', "\n"));
            let mut entries = vec![text, &single, &lines];
            entries.extend(text.split(['\n', '\r']));
            self.add(&entries);
            self.lose();
            // Secrets are short: edits inside a paste longer than a run holds are not followed. A shorter one is
            // followed as a multi-line field holds it (line breaks as line feeds), not as a single-line field would.
            if text.len() <= MAX_RUN {
                self.typed(&lines);
            }
        } else {
            self.keep(text);
            self.typed(text);
        }
    }

    /// Characters the person typed, inserted at the cursor (or, once it is lost, collected), up to [`MAX_RUN`].
    pub fn typed(&mut self, text: &str) {
        self.typing.insert(text);
        let Run { text, cursor } = &self.typing;
        if text.len() > MAX_RUN {
            // The characters just before the cursor go on into the next run, so that its pieces cover where they meet.
            let from = text[..*cursor].char_indices().rev().nth(MIN_LENGTH - 2).map_or(0, |(at, _)| at);
            let carried = text[from..*cursor].to_owned();
            self.lose();
            self.typing.insert(&carried);
        }
    }

    /// One edit of the typed run at its cursor. A move past either end of the run, Home or End, and a move to another
    /// line (see [`Edit::Line`]) may take the field's cursor outside it (the run may have started after a click in the
    /// middle of a value), to where the run cannot say what the field holds: the run ends there, kept with every piece
    /// of it (a later move may bring the cursor back into it), and what is typed next is a run of its own. A move or a
    /// deletion next to a character outside ASCII, which may be part of a cluster the page moves over or deletes whole
    /// (an emoji sequence, a letter and its marks), is not followed (see [`Redactions::lose`]). Once the cursor is
    /// lost, Backspace removes the last character collected, and any other edit starts another piece.
    ///
    /// A deletion may be undone later (Ctrl+Z, which the run does not follow), so every piece of the run as it stood is
    /// kept before each one: the deleted text stays hidden too.
    pub fn edit(&mut self, edit: Edit) {
        if matches!(edit, Edit::Backspace | Edit::Delete) {
            let run = self.typing.text.clone();
            self.add_pieces(&run);
        }
        if self.lost {
            let Run { text, cursor } = &mut self.typing;
            match edit {
                Edit::Backspace if *cursor > 0 => {
                    text.pop();
                    *cursor = text.len();
                }
                _ => self.lose(),
            }
            return;
        }
        match self.typing.edit(edit) {
            Followed::Unsure => self.lose(),
            Followed::Out => {
                let run = std::mem::take(&mut self.typing).text;
                self.keep_pieces(&run);
            }
            Followed::Within => self.undoable(),
            Followed::Deleted => {}
        }
    }

    /// Keeps the run as it stands before what an undo takes back whole: a paste,
    /// or what is typed after a move (an insertion inside the run, which leaves none of the run's pieces whole).
    fn undoable(&mut self) {
        let run = self.typing.text.clone();
        self.keep(&run);
    }

    /// An edit the run cannot follow (a shortcut, which may select, delete a word or undo; a selection; an edit next to
    /// a character outside ASCII): the run as it stood is kept with every piece of it, since the field may now hold any
    /// part of it, and until the next field boundary what is typed is collected without a cursor, each piece of it kept
    /// the same way once another key that is not a character ends it.
    pub fn lose(&mut self) {
        let run = std::mem::take(&mut self.typing).text;
        self.keep_pieces(&run);
        self.lost = true;
    }

    /// A field boundary (Enter, Tab, a click): the typed run so far becomes an entry, with
    /// every piece of it if the cursor was lost, and the next run is followed again.
    pub fn boundary(&mut self) {
        let run = std::mem::take(&mut self.typing).text;
        if std::mem::take(&mut self.lost) {
            self.keep_pieces(&run);
        } else {
            self.keep(&run);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Applies the contract to one agent observation (see [`Scrub::apply`]).
    #[cfg(test)]
    pub fn apply(&self, value: &mut Value) {
        self.scrub().apply(value);
    }
}

/// The contract for an agent's observations as the session's set stands, cheap to copy out of its lock.
#[derive(Clone, Debug)]
pub struct Scrub {
    entries: Arc<HashSet<String>>,
    /// Built over `entries` (see [`Scrub::build`]).
    matcher: Option<Arc<Matcher>>,
    screenshots_blocked: bool,
    /// The set could not hold what the person typed: nothing of an observation is passed on.
    hidden: bool,
}

/// What an observation becomes once the set is full: a tool result that says why, and nothing of the page.
const HIDDEN: &str = "[redacted]: the person typed more here than redaction can follow, so nothing of the page is \
     passed on in this session any more";

impl Scrub {
    /// Builds the matcher over the entries, unless it is built (or there are none): true when it was built here.
    pub fn build(&mut self) -> bool {
        if self.matcher.is_some() || self.entries.is_empty() {
            return false;
        }
        self.matcher = Some(Arc::new(Matcher::new(self.entries.iter())));
        true
    }

    /// Applies the contract to one agent observation, the names of its fields included (a script chooses them).
    pub fn apply(&mut self, value: &mut Value) {
        self.build();
        if self.hidden {
            *value = match value {
                Value::Object(_) => serde_json::json!({"content": [{"type": "text", "text": HIDDEN}], "isError": true}),
                _ => Value::String(HIDDEN.into()),
            };
            return;
        }
        self.filter(value);
    }

    /// Removes every recorded string, and every password field's value, from one text.
    ///
    /// Every match of every entry is found on the original text and overlapping matches are merged, so an entry
    /// inside a longer one (a user name, then a value that contains it) or two overlapping entries leave nothing.
    fn scrub(&self, s: &mut String) {
        if let Some(matcher) = &self.matcher {
            let covered = covered(matcher, s);
            if !covered.is_empty() {
                let mut out = String::with_capacity(s.len());
                let mut at = 0;
                for (start, end) in covered {
                    out.push_str(&s[at..start]);
                    out.push_str(MARK);
                    at = end;
                }
                out.push_str(&s[at..]);
                *s = out;
            }
        }
        if s.contains(PASSWORD) {
            *s = s.lines().map(redact_password_line).collect::<Vec<_>>().join("\n");
        }
    }

    fn filter(&self, value: &mut Value) {
        match value {
            Value::String(s) => self.scrub(s),
            Value::Array(items) => {
                if self.screenshots_blocked {
                    items.retain(|item| item.get("type").and_then(Value::as_str) != Some("image"));
                }
                items.iter_mut().for_each(|item| self.filter(item));
            }
            Value::Object(map) => {
                if self.screenshots_blocked {
                    map.retain(|key, _| !key.ends_with("_b64"));
                }
                // A password field as an element of its own (`tree_format: "elements"`), its value beside its role.
                if map.get("role").and_then(Value::as_str) == Some(PASSWORD) {
                    for key in ["value", "text"] {
                        if let Some(value) = map.get_mut(key) {
                            *value = Value::String(MARK.into());
                        }
                    }
                }
                let recorded = |key: &str| self.matcher.as_ref().is_some_and(|m| !covered(m, key).is_empty());
                if map.keys().any(|key| recorded(key) || key.contains(PASSWORD)) {
                    *map = std::mem::take(map)
                        .into_iter()
                        .map(|(mut key, item)| {
                            self.scrub(&mut key);
                            (key, item)
                        })
                        .collect();
                }
                map.values_mut().for_each(|item| self.filter(item));
            }
            // A number the person typed (a card number, a PIN) that a script returned as one.
            Value::Number(n) => {
                if self.matcher.as_ref().is_some_and(|m| !covered(m, &n.to_string()).is_empty()) {
                    *value = Value::String(MARK.into());
                }
            }
            Value::Null | Value::Bool(_) => {}
        }
    }
}

/// Every entry in one automaton over bytes (Aho-Corasick), each state holding the length of the longest entry that ends
/// there. Built in time and memory bounded by the entries' bytes, whatever they share: a state takes the longest entry
/// its failure link reaches, never a list of them.
#[derive(Debug)]
struct Matcher {
    /// Each state's transitions, sorted by byte: `edges[first[s]..first[s + 1]]`.
    first: Vec<u32>,
    edges: Vec<(u8, u32)>,
    fail: Vec<u32>,
    longest: Vec<u32>,
}

impl Matcher {
    fn new<'a>(entries: impl IntoIterator<Item = &'a String>) -> Matcher {
        let mut children: Vec<Vec<(u8, u32)>> = vec![Vec::new()];
        let mut longest = vec![0u32];
        for entry in entries {
            let mut state = 0;
            for &byte in entry.as_bytes() {
                state = match children[state].binary_search_by_key(&byte, |&(b, _)| b) {
                    Ok(at) => children[state][at].1 as usize,
                    Err(at) => {
                        children.push(Vec::new());
                        longest.push(0);
                        let next = children.len() - 1;
                        children[state].insert(at, (byte, next as u32));
                        next
                    }
                };
            }
            longest[state] = longest[state].max(entry.len() as u32);
        }
        let goto = |children: &[Vec<(u8, u32)>], state: usize, byte: u8| {
            children[state].binary_search_by_key(&byte, |&(b, _)| b).ok().map(|at| children[state][at].1)
        };
        // Breadth first, so that a state's failure link, which is shallower, is complete before it.
        let mut fail = vec![0u32; children.len()];
        let mut queue: VecDeque<u32> = children[0].iter().map(|&(_, next)| next).collect();
        while let Some(state) = queue.pop_front() {
            let state = state as usize;
            longest[state] = longest[state].max(longest[fail[state] as usize]);
            for &(byte, next) in &children[state] {
                let mut back = fail[state] as usize;
                fail[next as usize] = loop {
                    if let Some(to) = goto(&children, back, byte) {
                        break to;
                    }
                    if back == 0 {
                        break 0;
                    }
                    back = fail[back] as usize;
                };
                queue.push_back(next);
            }
        }
        let mut first = Vec::with_capacity(children.len() + 1);
        let mut edges = Vec::new();
        for state in &children {
            first.push(edges.len() as u32);
            edges.extend_from_slice(state);
        }
        first.push(edges.len() as u32);
        Matcher { first, edges, fail, longest }
    }

    fn step(&self, mut state: usize, byte: u8) -> usize {
        loop {
            let edges = &self.edges[self.first[state] as usize..self.first[state + 1] as usize];
            if let Ok(at) = edges.binary_search_by_key(&byte, |&(b, _)| b) {
                return edges[at].1 as usize;
            }
            if state == 0 {
                return 0;
            }
            state = self.fail[state] as usize;
        }
    }
}

/// The union of every match of every entry in `text`, as disjoint byte ranges in order, in one scan and in memory
/// bounded by the text and the entries, never by the matches: a text of one repeated byte holds a match of nearly
/// every entry at every position. Where several entries end at one position, the longest holds the others, so only it
/// is taken and merged at once into the ranges so far, which it can only extend.
fn covered(matcher: &Matcher, text: &str) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut state = 0;
    for (at, &byte) in text.as_bytes().iter().enumerate() {
        state = matcher.step(state, byte);
        let len = matcher.longest[state] as usize;
        if len == 0 {
            continue;
        }
        let end = at + 1;
        let mut start = end - len;
        while let Some(&(s, e)) = ranges.last()
            && e >= start
        {
            start = start.min(s);
            ranges.pop();
        }
        ranges.push((start, end));
    }
    ranges
}

/// Text typed into a field, as the field holds it after the edits followed, and where in it (a byte offset, on a
/// character boundary) the field's cursor stands.
#[derive(Default, Debug, Clone)]
struct Run {
    text: String,
    cursor: usize,
}

/// What one edit did to a [`Run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Followed {
    /// The cursor is still in the run: it moved within it, or a deletion at one of its ends removed what lies past it.
    Within,
    /// A move past either end of the run, Home or End, or a move to another line, which may take the field's cursor
    /// outside it (the run may have started after a click in the middle of a value).
    Out,
    /// A character removed: the text on either side of it now meets at the cursor.
    Deleted,
    /// A move or a deletion next to a character outside ASCII, which may be part of a cluster the page moves over or
    /// deletes whole (an emoji sequence, a letter and its marks): not applied.
    Unsure,
}

impl Run {
    /// Text typed at the cursor.
    fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// One edit at the cursor, applied as the field applies it unless it is [`Followed::Unsure`].
    fn edit(&mut self, edit: Edit) -> Followed {
        let previous = self.text[..self.cursor].chars().next_back();
        let mut ahead = self.text[self.cursor..].chars();
        let (next, after_next) = (ahead.next(), ahead.next());
        let wide = |c: Option<char>| c.is_some_and(|c| !c.is_ascii());
        let unsure = match edit {
            Edit::Left | Edit::Backspace => wide(previous),
            Edit::Right | Edit::Delete => wide(next) || wide(after_next),
            Edit::Home | Edit::End | Edit::Line => false,
        };
        if unsure {
            return Followed::Unsure;
        }
        let before = previous.map_or(0, char::len_utf8);
        let after = next.map_or(0, char::len_utf8);
        match edit {
            Edit::Left if before > 0 => self.cursor -= before,
            Edit::Right if after > 0 => self.cursor += after,
            Edit::Left | Edit::Right | Edit::Home | Edit::End | Edit::Line => return Followed::Out,
            Edit::Backspace if before > 0 => {
                self.cursor -= before;
                self.text.drain(self.cursor..self.cursor + before);
                return Followed::Deleted;
            }
            Edit::Delete if after > 0 => {
                self.text.drain(self.cursor..self.cursor + after);
                return Followed::Deleted;
            }
            // At an end of the run: the field deletes what lies past it, and the cursor stays where it was.
            Edit::Backspace | Edit::Delete => {}
        }
        Followed::Within
    }
}

/// An edit of the typed run at its cursor (see [`Redactions::edit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Left,
    Right,
    Home,
    End,
    /// Up, Down, Page Up or Page Down, or a keypad key (a move or a digit, as the remote Num Lock has it): to another
    /// line of a multi-line field, or to an end of a single-line one.
    Line,
    Backspace,
    Delete,
}

/// A password field's accessibility role.
const PASSWORD: &str = "password text";

/// An accessibility-tree line for a password field keeps its role and loses whatever follows it; the role alone (an
/// element's `role`) stays as it is.
fn redact_password_line(line: &str) -> String {
    match line.find(PASSWORD) {
        Some(at) if at + PASSWORD.len() < line.len() => format!("{}{PASSWORD} \"{MARK}\"", &line[..at]),
        _ => line.to_owned(),
    }
}

/// The US layout's rows of characters from their first evdev key code, unshifted and shifted.
const ROWS: [(u16, &str, &str); 4] = [
    (2, "1234567890-=", "!@#$%^&*()_+"),
    (16, "qwertyuiop[]", "QWERTYUIOP{}"),
    (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
    (44, "zxcvbnm,./", "ZXCVBNM<>?"),
];

/// US-layout characters of evdev key codes, unshifted and shifted, for the typed-run record.
pub fn evdev_char(code: u16, shift: bool) -> Option<char> {
    if code == 57 {
        return Some(' ');
    }
    if code == 43 {
        return Some(if shift { '|' } else { '\\' });
    }
    ROWS.iter().find_map(|(start, plain, shifted)| {
        let index = code.checked_sub(*start)? as usize;
        let row = if shift { shifted } else { plain };
        row.chars().nth(index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The matcher an agent call builds over the set as it stands.
    fn built(r: &Redactions) -> Arc<Matcher> {
        let mut scrub = r.scrub();
        assert!(scrub.build());
        scrub.matcher.unwrap()
    }

    #[test]
    fn a_matcher_built_out_of_the_set_is_kept_only_if_nothing_was_recorded_since() {
        let mut r = Redactions::default();
        r.keep("hunter2-secret");
        let mut early = r.scrub();
        // Recorded while an agent call builds the matcher out of the session's lock: that matcher lacks it.
        r.keep("pasted-token-123");
        assert!(early.build());
        r.install(&early);
        assert!(r.matcher.is_none(), "not kept");
        let mut current = r.scrub();
        assert!(current.build());
        r.install(&current);
        assert!(r.matcher.is_some() && !r.scrub().build(), "the next call finds it built");
        let mut observation = json!("hunter2-secret pasted-token-123");
        r.scrub().apply(&mut observation);
        assert_eq!(observation, "[redacted] [redacted]");
    }

    #[test]
    fn typed_and_pasted_strings_leave_every_later_observation() {
        let mut r = Redactions::default();
        r.typed("hunter2-secret");
        r.boundary();
        r.keep("pasted-token-123");
        r.keep("ab");
        let mut observation = json!({
            "tree_markdown": "- entry \"Password\" value hunter2-secret\n- password text \"anything\"",
            "items": [{"type": "text", "text": "token pasted-token-123 and ab"}],
        });
        r.apply(&mut observation);
        let text = observation.to_string();
        assert!(!text.contains("hunter2-secret") && !text.contains("pasted-token-123") && !text.contains("anything"));
        assert!(text.contains("and ab"), "strings under the minimum length stay");
    }

    #[test]
    fn a_value_inside_json_text_is_hidden_too() {
        let mut r = Redactions::default();
        r.typed("Tr0ub\"4dor");
        r.boundary();
        r.pasted("C:\\keys\tfile");
        r.boundary();
        // run_script's text item carries its returned value as JSON, beside the parsed copy.
        let text = json!(["Tr0ub\"4dor", "C:\\keys\tfile"]).to_string();
        let mut result =
            json!({"content": [{"type": "text", "text": text}], "structuredContent": {"result": ["Tr0ub\"4dor"]}});
        r.apply(&mut result);
        assert_eq!(result["content"][0]["text"], "[\"[redacted]\",\"[redacted]\"]");
        assert_eq!(result["structuredContent"]["result"][0], "[redacted]");
    }

    #[test]
    fn a_number_that_holds_a_recorded_string_is_hidden() {
        let mut r = Redactions::default();
        r.typed("4111111111111111");
        r.boundary();
        let mut result = json!({"structuredContent": {"result": [4111111111111111u64, 4111, 3.5]}});
        r.apply(&mut result);
        assert_eq!(result["structuredContent"]["result"], json!(["[redacted]", 4111, 3.5]));
    }

    #[test]
    fn a_password_field_in_the_elements_format_loses_its_value() {
        let r = Redactions::default();
        let mut result = json!({"elements": [
            {"role": "password text", "label": "Password", "value": "autofilled-pass"},
            {"role": "entry", "label": "User", "value": "alice"},
        ]});
        r.apply(&mut result);
        assert_eq!(result["elements"][0], json!({"role": "password text", "label": "Password", "value": "[redacted]"}));
        assert_eq!(result["elements"][1]["value"], "alice");
    }

    #[test]
    fn a_recorded_string_leaves_the_names_of_fields_too() {
        let mut r = Redactions::default();
        r.keep("hunter2-secret");
        let mut observation = json!({"hunter2-secret": true, "kept": {"x hunter2-secret": 1}});
        r.apply(&mut observation);
        assert!(!observation.to_string().contains("hunter2-secret"), "{observation}");
        assert_eq!(observation["kept"]["x [redacted]"], 1);
    }

    #[test]
    fn an_entry_inside_a_longer_one_hides_all_of_it() {
        let mut r = Redactions::default();
        r.typed("alice");
        r.boundary();
        r.typed("alice-recovery-7731");
        r.boundary();
        let mut observation = json!("value alice-recovery-7731 and alice");
        r.apply(&mut observation);
        assert_eq!(observation, "value [redacted] and [redacted]");
    }

    #[test]
    fn overlapping_entries_hide_their_union() {
        let mut r = Redactions::default();
        r.keep("abcd");
        r.keep("cdef");
        let mut observation = json!({"abcdef x": "abcdef"});
        r.apply(&mut observation);
        assert_eq!(observation, json!({"[redacted] x": "[redacted]"}));
    }

    #[test]
    fn a_text_holding_a_match_of_every_entry_at_every_position_is_scrubbed_in_bounded_time_and_memory() {
        const ENTRIES: usize = 64;
        let mut r = Redactions::default();
        for len in MIN_LENGTH..MIN_LENGTH + ENTRIES {
            r.keep(&"a".repeat(len));
        }
        assert!(!r.saturated && r.len() == ENTRIES);
        let matcher = built(&r);
        // Some 64 million overlapping matches, of which only the longest at each position is taken, merged at once.
        let text = "a".repeat(1 << 20);
        let started = std::time::Instant::now();
        let ranges = covered(&matcher, &text);
        assert_eq!(ranges, [(0, text.len())]);
        assert!(ranges.capacity() <= 4, "the ranges never held more than their union: {}", ranges.capacity());
        let mut observation = json!(format!("x {text} y"));
        r.apply(&mut observation);
        assert_eq!(observation, "x [redacted] y");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn the_union_is_that_of_every_overlapping_match() {
        // Short entries over two letters, so they overlap and nest in every way, against every match collected.
        let mut seed = 0x2545_f491_u32;
        let mut next = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) % n
        };
        let mut word = |len: u32| (0..len).map(|_| if next(2) == 0 { 'a' } else { 'b' }).collect::<String>();
        for round in 0..200 {
            let mut r = Redactions::default();
            for i in 0..1 + round % 5 {
                r.keep(&word(MIN_LENGTH as u32 + (round + i) % 4));
            }
            let text = word(64);
            let all = aho_corasick::AhoCorasick::new(r.entries.iter()).unwrap();
            let mut expected: Vec<(usize, usize)> = Vec::new();
            let mut found: Vec<_> = all.find_overlapping_iter(&text).map(|m| (m.start(), m.end())).collect();
            found.sort_unstable();
            for (start, end) in found {
                match expected.last_mut() {
                    Some(last) if start <= last.1 => last.1 = last.1.max(end),
                    _ => expected.push((start, end)),
                }
            }
            assert_eq!(covered(&built(&r), &text), expected, "{:?} in {text}", r.entries);
        }
    }

    #[test]
    fn ranges_that_only_touch_merge_and_apart_ones_stay_apart() {
        let mut r = Redactions::default();
        r.keep("abcd");
        r.keep("efgh");
        assert_eq!(covered(&built(&r), "abcdefgh abcd"), [(0, 8), (9, 13)]);
    }

    #[test]
    fn screenshots_stop_after_a_takeover() {
        let mut r = Redactions::default();
        let mut before = json!({"content": [{"type": "image", "data": "AAAA"}, {"type": "text", "text": "ok"}], "screenshot_png_b64": "AAAA"});
        r.apply(&mut before);
        assert!(before.to_string().contains("AAAA"));
        r.screenshots_blocked = true;
        let mut after = before.clone();
        r.apply(&mut after);
        assert!(!after.to_string().contains("AAAA") && after.to_string().contains("ok"));
    }

    #[test]
    fn a_run_whose_cursor_was_lost_is_hidden_piece_by_piece_counting_against_the_bytes_only() {
        let mut r = Redactions::default();
        r.typed("correct horse battery staple");
        r.lose();
        r.typed("pencil");
        r.boundary();
        assert!(!r.lost);
        assert_eq!(r.len(), 2 + "correct horse battery staple".len() - 3 + "pencil".len() - 3);
        let mut observation = json!("correct horse battery pencil; a staple; xyz");
        r.apply(&mut observation);
        assert_eq!(observation, "[redacted]; a[redacted]; xyz", "\" sta\" is a piece too");
    }

    #[test]
    fn what_a_deletion_removed_stays_hidden_should_an_undo_bring_it_back() {
        for lost in [false, true] {
            let mut r = Redactions::default();
            if lost {
                r.lose();
            }
            r.typed("\u{5bc6}\u{7801}\u{673a}\u{5bc6}");
            for _ in 0..4 {
                r.edit(Edit::Backspace);
            }
            r.typed("ok");
            r.boundary();
            let mut observation = json!("\u{5bc6}\u{7801}\u{673a}\u{5bc6}");
            r.apply(&mut observation);
            assert_eq!(observation, "[redacted]", "lost: {lost}");
        }
    }

    /// Printable characters whose windows of [`MIN_LENGTH`] are all distinct (with all but certainty).
    fn distinct(count: usize, seed: &mut u64) -> String {
        (0..count)
            .map(|_| {
                *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                char::from(b'!' + ((*seed >> 33) % 94) as u8)
            })
            .collect()
    }

    #[test]
    fn a_keystroke_costs_at_most_the_pieces_of_one_bounded_run_whatever_was_typed_before() {
        let mut r = Redactions::default();
        let mut seed = 7;
        // A long run of distinct pieces, then Backspace and a character typed again, in turn: each Backspace keeps
        // every piece of the run, which never holds more than MAX_RUN bytes, and none of it builds the matcher.
        for c in distinct(16_387, &mut seed).chars() {
            r.typed(&c.to_string());
            assert!(r.typing.text.len() <= MAX_RUN);
        }
        let started = std::time::Instant::now();
        for c in distinct(20_000, &mut seed).chars() {
            r.edit(Edit::Backspace);
            r.typed(&c.to_string());
            assert!(r.typing.text.len() <= MAX_RUN && r.matcher.is_none());
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn a_run_past_its_bound_stays_hidden_whole_and_after_edits() {
        let mut seed = 11;
        let long = distinct(3 * MAX_RUN + 5, &mut seed);
        let mut r = Redactions::default();
        r.typed(&long);
        r.edit(Edit::Backspace);
        r.boundary();
        assert!(!r.saturated);
        let mut observation = json!([long.clone(), long[..long.len() - 1].to_owned(), long[MAX_RUN / 2..].to_owned()]);
        r.apply(&mut observation);
        assert_eq!(observation, json!(["[redacted]", "[redacted]", "[redacted]"]));
        // Typed a character at a time, then a move back into it and a deletion there.
        let mut r = Redactions::default();
        long.chars().for_each(|c| r.typed(&c.to_string()));
        (0..MAX_RUN + 9).for_each(|_| r.edit(Edit::Left));
        r.edit(Edit::Backspace);
        r.typed("XYZW");
        r.boundary();
        let at = long.len() - MAX_RUN - 10;
        let mut observation = json!(format!("{}XYZW{}", &long[..at], &long[at + 1..]));
        r.apply(&mut observation);
        assert_eq!(observation, "[redacted]");
    }

    #[test]
    fn a_64_kib_paste_is_kept_whole_and_give_back_leaves_it_hidden() {
        let mut seed = 3;
        let paste = distinct(64 * 1024, &mut seed);
        let mut r = Redactions::default();
        r.typed("before");
        r.pasted(&paste);
        r.boundary();
        // Kept whole, as it is and as JSON text holds it (its quotes and backslashes escaped), never piece by piece.
        let escaped = serde_json::to_string(&paste).unwrap().len() - 2;
        assert!(!r.saturated && r.bytes < paste.len() + escaped + MAX_RUN, "{}", r.bytes);
        let mut observation = json!(format!("value before{paste} end"));
        r.apply(&mut observation);
        assert_eq!(observation, "value [redacted] end");
    }

    #[test]
    fn a_long_paste_stays_hidden_as_a_single_line_field_holds_it() {
        let mut r = Redactions::default();
        r.pasted(&format!("sec{}ret", "\n".repeat(256)));
        r.boundary();
        let mut observation = json!("value secret end");
        r.apply(&mut observation);
        assert_eq!(observation, "value [redacted] end");
    }

    #[test]
    fn a_long_paste_stays_hidden_line_by_line() {
        // Edits inside a paste longer than a run holds are not followed: here Up, Home and Delete in a multi-line field
        // change its first line, and the other lines stay hidden.
        let (first, second) = (format!("secret{}", "x".repeat(250)), "z".repeat(256));
        let paste = format!("{first}\n{second}");
        let mut r = Redactions::default();
        r.pasted(&paste);
        r.boundary();
        r.edit(Edit::Home);
        r.edit(Edit::Delete);
        r.boundary();
        let mut observation = json!([paste, second, format!("{first}\r"), first.replace('\n', "")]);
        r.apply(&mut observation);
        assert_eq!(observation, json!(["[redacted]", "[redacted]", "[redacted]\r", "[redacted]"]));
    }

    #[test]
    fn a_short_multi_line_paste_stays_hidden_after_backspace_and_a_carriage_return_as_a_line_feed() {
        for paste in ["sec\nretx", "sec\rretx", "sec\r\nretx"] {
            let mut r = Redactions::default();
            r.pasted(paste);
            r.edit(Edit::Backspace);
            r.boundary();
            let mut observation = json!(["sec\nret", "secret"]);
            r.apply(&mut observation);
            assert_eq!(observation, json!(["[redacted]", "[redacted]"]), "{paste:?}");
        }
        let paste = format!("hunter2\r{}", "x".repeat(249));
        let mut r = Redactions::default();
        r.pasted(&paste);
        r.boundary();
        let mut observation = json!([paste.replace('\r', "\n"), "hunter2"]);
        r.apply(&mut observation);
        assert_eq!(observation, json!(["[redacted]", "[redacted]"]));
    }

    #[test]
    fn a_multi_line_paste_stays_hidden_in_either_kind_of_field() {
        let mut r = Redactions::default();
        r.pasted("123\n456");
        r.boundary();
        let mut observation = json!(["123\n456", "123456"]);
        r.apply(&mut observation);
        assert_eq!(observation, json!(["[redacted]", "[redacted]"]));
    }

    #[test]
    fn the_matcher_holds_at_most_a_state_per_byte_recorded_whatever_the_entries_share() {
        // Entries that are suffixes of one another, which an automaton listing every match a state reaches would
        // multiply into tens of millions of records.
        let mut r = Redactions::default();
        (MIN_LENGTH..=256).for_each(|len| r.keep(&"a".repeat(len)));
        (0..880).for_each(|n| r.keep(&format!("{n:04X}{}", "a".repeat(252))));
        assert!(!r.saturated);
        let matcher = built(&r);
        assert!(matcher.fail.len() <= r.bytes + 1 && matcher.edges.len() < matcher.fail.len());
        let mut observation = json!(format!("x 00A1{} y", "a".repeat(300)));
        r.apply(&mut observation);
        assert_eq!(observation, "x [redacted] y");
    }

    #[test]
    fn a_full_set_hides_every_observation_whole() {
        let mut r = Redactions::default();
        let mut seed = 5;
        while !r.saturated {
            r.keep(&distinct(1024, &mut seed));
        }
        assert!(r.bytes <= MAX_BYTES);
        let mut result = json!({"content": [{"type": "text", "text": "an ordinary page"}]});
        r.apply(&mut result);
        assert_eq!(result["isError"], true);
        assert!(!result.to_string().contains("ordinary"), "{result}");
        let mut text = json!("an ordinary error");
        r.apply(&mut text);
        assert!(!text.to_string().contains("ordinary"), "{text}");
    }

    #[test]
    fn evdev_codes_map_to_us_characters() {
        assert_eq!(evdev_char(30, false), Some('a'));
        assert_eq!(evdev_char(30, true), Some('A'));
        assert_eq!(evdev_char(2, true), Some('!'));
        assert_eq!(evdev_char(57, false), Some(' '));
        assert_eq!(evdev_char(28, false), None);
    }
}
