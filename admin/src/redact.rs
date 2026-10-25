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

use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use serde_json::Value;

/// What the set holds: far more than a person types, and a paste of at most 64 KiB in every form [`Redactions::pasted`]
/// keeps of it (four at most, each no longer than the paste), with a matcher the agent's next call builds in tens of
/// milliseconds.
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

#[derive(Default, Debug)]
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

    /// Adds `texts` to the set, in time bounded by their length, up to [`MAX_BYTES`]. Each is kept once, as it is: the
    /// matcher finds it escaped in JSON text too (see [`covered`]).
    fn add(&mut self, texts: &[&str]) {
        for text in texts {
            let text = text.trim_matches(|c: char| c == '\n' || c == '\r');
            if text.chars().count() < MIN_LENGTH || self.entries.contains(text) {
                continue;
            }
            if self.bytes + text.len() > MAX_BYTES {
                self.saturated = true;
                return;
            }
            Arc::make_mut(&mut self.entries).insert(text.to_owned());
            self.bytes += text.len();
            self.matcher = None;
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
            Followed::Out => self.end_run(true),
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
        self.end_run(true);
        self.lost = true;
    }

    /// A field boundary (Enter, Tab, Escape, Shift+Insert, a click): the typed run so far becomes an entry, with every
    /// piece of it if the cursor was lost, and the next run is followed again.
    ///
    /// The scope: redaction hides what the person typed or pasted, and follows their edits while they type a value,
    /// until they leave it here or with a move out of the run. An edit of that value made after it (Enter that keeps
    /// the field focused, then Backspace) is not followed, and is out of scope.
    pub fn boundary(&mut self) {
        let lost = std::mem::take(&mut self.lost);
        self.end_run(lost);
    }

    /// Ends the typed run: it is kept, with every piece of it if `pieces`, and the next one starts empty.
    fn end_run(&mut self, pieces: bool) {
        let run = std::mem::take(&mut self.typing).text;
        if pieces {
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

/// The contract for an agent's observations as the session's set stands, cheap to take out of its lock.
#[derive(Debug)]
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
    /// Builds the matcher over the entries, unless it is built, there are none, or the set is full and nothing of an
    /// observation is passed on: true when it was built here.
    pub fn build(&mut self) -> bool {
        if self.matcher.is_some() || self.entries.is_empty() || self.hidden {
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

    /// Removes every password field's value and every recorded string from one text.
    ///
    /// Both are found on the text as written, so neither can hide the other: a recorded string may be a field's name,
    /// and a recorded value may hold a password field's role. Every match of every entry is found on the text as it is
    /// and with its JSON escapes decoded (see [`covered`]). Overlapping ranges are merged into one mark, so an entry
    /// inside a longer one (a user name, then a value that contains it), two overlapping entries, or an entry across a
    /// password field's value leave nothing. A password field's value, alone or around a recorded string, keeps its own
    /// replacement, and the rest of the text stays as it was written.
    fn scrub(&self, s: &mut String) {
        if let Some(scrubbed) = scrub_text(self.matcher.as_deref(), s, 0, 0) {
            *s = scrubbed;
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
                let recorded =
                    |key: &str| self.matcher.as_ref().is_some_and(|m| !covered(m, key, MAX_LEVELS).is_empty());
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
                if self.matcher.as_ref().is_some_and(|m| !covered(m, &n.to_string(), MAX_LEVELS).is_empty()) {
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
    /// The root's transition on each byte, read directly: most bytes of an observation are read at the root, where a
    /// search of its edges took 2.5 times as long.
    root: [u32; 256],
    /// The longest entry's length.
    widest: usize,
}

impl Matcher {
    fn new<'a>(entries: impl IntoIterator<Item = &'a String>) -> Matcher {
        let mut children: Vec<Vec<(u8, u32)>> = vec![Vec::new()];
        let mut longest = vec![0u32];
        let mut widest = 0;
        for entry in entries {
            widest = widest.max(entry.len());
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
        let mut root = [0; 256];
        children[0].iter().for_each(|&(byte, next)| root[usize::from(byte)] = next);
        Matcher { first, edges, fail, longest, root, widest }
    }

    fn step(&self, mut state: usize, byte: u8) -> usize {
        while state != 0 {
            let edges = &self.edges[self.first[state] as usize..self.first[state + 1] as usize];
            if let Ok(at) = edges.binary_search_by_key(&byte, |&(b, _)| b) {
                return edges[at].1 as usize;
            }
            state = self.fail[state] as usize;
        }
        self.root[usize::from(byte)] as usize
    }
}

/// How many times an observation's JSON escapes are decoded, by [`covered`] and by [`scrub_text`] for a JSON string
/// that holds a password field's role: a value in JSON text that a script returned as JSON text is escaped twice, and
/// three times inside that. A value escaped more often is the stated residual of one the page transformed.
const MAX_LEVELS: usize = 3;

/// The union of every match of every entry in `text`: in the text as it is and, while it holds a backslash, with its
/// JSON escapes decoded once, then again, up to `most` times, each match taken back to the bytes of `text` it was
/// decoded from. Each level is one more scan of the text. A range never splits a character, nor an escape of the text
/// or of a level decoded (a match of `nabc` in `\\nabc` would split the `\n` it holds decoded once), so one inside a
/// JSON string leaves it valid JSON, and so does JSON text inside one.
fn covered(matcher: &Matcher, text: &str, most: usize) -> Covered {
    let mut covered = Covered::new(text.len());
    let (mut levels, mut deeper) = (0, covered_in(matcher, written(text), text.len(), &mut covered));
    while deeper && levels < most {
        levels += 1;
        // Each level is a type of its own rather than a boxed iterator, whose call through a pointer for each byte and
        // layer took a seventh of the whole search of a desktop state's JSON text. Only the last layer says whether it
        // decoded an escape.
        let escaped = Cell::new(false);
        let last = Some(&escaped);
        let backslash = match levels {
            1 => covered_in(matcher, Unescaped::new(written(text), last), text.len(), &mut covered),
            2 => covered_in(matcher, Unescaped::new(once(text), last), text.len(), &mut covered),
            3 => covered_in(matcher, Unescaped::new(Unescaped::new(once(text), None), last), text.len(), &mut covered),
            _ => unreachable!("an observation is decoded at most MAX_LEVELS times"),
        };
        // Decoding once more changes nothing unless this level decoded an escape and still holds a backslash.
        deeper = escaped.get() && backslash;
    }
    // The escapes the last level holds, which no level decoded (JSON text in a string decoded `most` times): a match
    // found in part of one takes all of it too, in one more pass that only widens. Its layers are boxed: it runs only
    // once a match was found in such a text.
    if deeper && !covered.is_empty() {
        let mut bytes: Box<dyn Iterator<Item = Byte> + '_> = Box::new(written(text));
        for _ in 0..=levels {
            bytes = Box::new(Unescaped::new(bytes, None));
        }
        for (_, from, to) in bytes {
            if to - from > 1 && covered.partly(from, to) {
                covered.set(from, to);
            }
        }
    }
    covered
}

/// The bytes of `text` with its JSON escapes decoded once.
fn once(text: &str) -> Unescaped<'static, impl Iterator<Item = Byte> + '_> {
    Unescaped::new(written(text), None)
}

/// The bytes of a text that matches cover, one bit each: what a scrub holds of its matches is bounded by the text, never
/// by how many matches it holds (a text of a short entry and a space, repeated, holds one every five bytes). Ranges
/// that overlap or only touch are one run of it, as they are one mark.
#[derive(Debug)]
struct Covered {
    words: Vec<u64>,
    len: usize,
}

impl Covered {
    fn new(len: usize) -> Covered {
        Covered { words: vec![0; len.div_ceil(64)], len }
    }

    /// Covers the bytes from `start` to `end`.
    fn set(&mut self, start: usize, end: usize) {
        let mut at = start;
        while at < end {
            let (word, bit) = (at / 64, at % 64);
            let bits = (64 - bit).min(end - at);
            self.words[word] |= (u64::MAX >> (64 - bits)) << bit;
            at += bits;
        }
    }

    /// The first byte from `at` on that is covered (or, with `covered` false, that is not), or the text's length.
    fn find(&self, mut at: usize, covered: bool) -> usize {
        while at < self.len {
            let word = self.words[at / 64];
            let word = if covered { word } else { !word } >> (at % 64);
            if word != 0 {
                return (at + word.trailing_zeros() as usize).min(self.len);
            }
            at = (at / 64 + 1) * 64;
        }
        self.len
    }

    /// The covered ranges, apart and in order.
    fn runs(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let mut at = 0;
        std::iter::from_fn(move || {
            let start = self.find(at, true);
            (start < self.len).then(|| {
                at = self.find(start, false);
                (start, at)
            })
        })
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|&word| word == 0)
    }

    /// Whether some of the bytes from `start` to `end` are covered, and some not. Only those bytes are read (an
    /// escape's few): a search for the next covered byte would read to the end of an uncovered text at every escape.
    fn partly(&self, start: usize, end: usize) -> bool {
        let covered = |at: usize| self.words[at / 64] >> (at % 64) & 1 == 1;
        let first = covered(start);
        (start + 1..end).any(|at| covered(at) != first)
    }
}

/// A byte of a text as one level of decoding sees it, and the range of the original text it was decoded from.
type Byte = (u8, usize, usize);

/// The bytes of `text` as written, each with the range of the escape it is part of (a backslash and the character
/// after it, or `\u` and four hex digits, a surrogate pair as one), or else its own: a match that starts or ends inside
/// an escape then covers all of it, and never leaves a lone backslash before its mark (the `\` of `"x\nabc"`, for a
/// recorded `nabc`).
fn written(text: &str) -> impl Iterator<Item = Byte> + '_ {
    let bytes = text.as_bytes();
    let mut escape = (0, 0);
    bytes.iter().enumerate().map(move |(at, &byte)| {
        if at >= escape.1 {
            escape = (at, at + escape_length(bytes, at));
        }
        (byte, escape.0, escape.1)
    })
}

/// The length of the escape at `at` in `bytes`, or 1 when none starts there.
fn escape_length(bytes: &[u8], at: usize) -> usize {
    let unit = |at: usize| {
        let digits = bytes.get(at..at + 4).filter(|digits| digits.iter().all(u8::is_ascii_hexdigit))?;
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    };
    if bytes[at] != b'\\' {
        return 1;
    }
    match bytes.get(at + 1) {
        Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => 2,
        Some(b'u') => match unit(at + 2) {
            Some(0xd800..0xdc00)
                if bytes.get(at + 6..at + 8) == Some(b"\\u")
                    && unit(at + 8).is_some_and(|low| (0xdc00..0xe000).contains(&low)) =>
            {
                12
            }
            Some(_) => 6,
            None => 1,
        },
        _ => 1,
    }
}

/// Covers every match of every entry in `bytes`, decoded from a text of `len` bytes, in one scan and in memory bounded by
/// the longest entry besides `covered`, never by the matches: a text of one repeated byte holds a match of nearly every
/// entry at every position. Where several entries end at one position, the longest holds the others, so only it is
/// taken and merged at once into the open ranges, which it can only extend. A range no later match can reach is
/// covered and let go: a match starts no earlier than the oldest byte the last ones, at least as many as the longest
/// entry, hold. Returns whether `bytes` held a backslash.
fn covered_in(matcher: &Matcher, bytes: impl Iterator<Item = Byte>, len: usize, covered: &mut Covered) -> bool {
    let mut open: VecDeque<(usize, usize)> = VecDeque::new();
    // Where each of the last bytes, at least as many as the longest entry, starts in the text: a power of two of them,
    // so that a mask finds one rather than a division at every byte.
    let mask = matcher.widest.min(len).max(1).next_power_of_two() - 1;
    let mut starts = vec![0; mask + 1];
    let (mut state, mut backslash) = (0, false);
    for (at, (byte, from, to)) in bytes.enumerate() {
        backslash |= byte == b'\\';
        // A byte of an escape (of the text, or of the level this one decoded): a match an earlier level found in part
        // of it takes all of it. The earlier levels' matches are all in `covered` by now, and this level's, made of its
        // bytes, cover whole ones.
        if to - from > 1 && covered.partly(from, to) {
            covered.set(from, to);
        }
        starts[at & mask] = from;
        state = matcher.step(state, byte);
        let found = matcher.longest[state] as usize;
        if found == 0 {
            continue;
        }
        let mut start = starts[(at + 1 - found) & mask];
        while let Some(&(s, e)) = open.back()
            && e >= start
        {
            start = start.min(s);
            open.pop_back();
        }
        open.push_back((start, to));
        let oldest = starts[(at + 1) & mask];
        while let Some(&(s, e)) = open.front()
            && e < oldest
        {
            covered.set(s, e);
            open.pop_front();
        }
    }
    open.into_iter().for_each(|(s, e)| covered.set(s, e));
    backslash
}

/// The bytes of a text with each of its JSON escapes decoded (`\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t` and
/// `\u` with four hex digits, a surrogate pair as one character), each decoded byte coming from the whole escape. A
/// backslash that starts no escape stays as it is.
struct Unescaped<'a, I> {
    inner: I,
    /// Bytes read past an escape that turned out not to be one, to read again.
    ahead: VecDeque<Byte>,
    /// The rest of a character decoded from `\u`.
    out: VecDeque<Byte>,
    /// Set once an escape was decoded.
    escaped: Option<&'a Cell<bool>>,
}

impl<'a, I: Iterator<Item = Byte>> Unescaped<'a, I> {
    fn new(inner: I, escaped: Option<&'a Cell<bool>>) -> Self {
        Unescaped { inner, ahead: VecDeque::new(), out: VecDeque::new(), escaped }
    }

    fn pull(&mut self) -> Option<Byte> {
        self.ahead.pop_front().or_else(|| self.inner.next())
    }

    /// Four hex digits after `\u`, read: their value and the end of the last, or none, with what was read put back.
    fn unit(&mut self, read: &mut Vec<Byte>) -> Option<(u32, usize)> {
        let mut unit = 0;
        for _ in 0..4 {
            let byte = self.pull()?;
            read.push(byte);
            unit = unit * 16 + char::from(byte.0).to_digit(16)?;
        }
        Some((unit, read.last()?.2))
    }

    /// The character of a `\u` escape whose backslash and `u` are `read`, or none, with `read` holding all it read.
    fn unicode(&mut self, read: &mut Vec<Byte>) -> Option<(char, usize)> {
        let (high, end) = self.unit(read)?;
        if !(0xd800..0xdc00).contains(&high) {
            return Some((char::from_u32(high)?, end));
        }
        for expected in *b"\\u" {
            let byte = self.pull()?;
            read.push(byte);
            if byte.0 != expected {
                return None;
            }
        }
        let (low, end) = self.unit(read)?;
        if !(0xdc00..0xe000).contains(&low) {
            return None;
        }
        Some((char::from_u32(0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00))?, end))
    }
}

impl<I: Iterator<Item = Byte>> Iterator for Unescaped<'_, I> {
    type Item = Byte;

    fn next(&mut self) -> Option<Byte> {
        if let Some(byte) = self.out.pop_front() {
            return Some(byte);
        }
        let backslash = self.pull()?;
        if backslash.0 != b'\\' {
            return Some(backslash);
        }
        let Some(code) = self.pull() else { return Some(backslash) };
        let simple = match code.0 {
            b'"' | b'\\' | b'/' => Some(code.0),
            b'b' => Some(8),
            b'f' => Some(12),
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            _ => None,
        };
        let decoded = if let Some(byte) = simple {
            Some((byte, backslash.1, code.2))
        } else if code.0 == b'u' {
            let mut read = vec![code];
            match self.unicode(&mut read) {
                Some((c, end)) => {
                    let mut utf8 = [0; 4];
                    self.out.extend(c.encode_utf8(&mut utf8).bytes().map(|byte| (byte, backslash.1, end)));
                    self.out.pop_front()
                }
                None => {
                    read.into_iter().rev().for_each(|byte| self.ahead.push_front(byte));
                    None
                }
            }
        } else {
            self.ahead.push_front(code);
            None
        };
        match decoded {
            Some(byte) => {
                if let Some(escaped) = self.escaped {
                    escaped.set(true);
                }
                Some(byte)
            }
            None => Some(backslash),
        }
    }
}

/// The deepest nesting of JSON text [`password_edits`] follows; deeper text is handled as text that is not JSON.
const MAX_DEPTH: usize = 128;

/// `text` with every password field's value and every recorded string in it replaced (see [`Scrub::scrub`]), or none
/// if it holds neither. `text` is an observation's string decoded `levels` times already (see [`MAX_LEVELS`]), so its
/// own escapes are decoded only as many more times.
fn scrub_text(matcher: Option<&Matcher>, text: &str, depth: usize, levels: usize) -> Option<String> {
    let edits = password_edits(matcher, text, depth, levels);
    let found = matcher.map(|matcher| covered(matcher, text, MAX_LEVELS - levels));
    // A whole string the walk scrubbed on its own, decoded, holds no match of its own any more. Strictly inside: a
    // match that reaches a string's quotes is not one the string's own scrub could find.
    let wholes: Vec<(usize, usize)> = edits.iter().filter(|e| e.3).map(|&(s, e, ..)| (s, e)).collect();
    let inside = |&(start, end): &(usize, usize)| {
        let at = wholes.partition_point(|&(s, _)| s < start);
        at > 0 && wholes[at - 1].0 < start && end < wholes[at - 1].1
    };
    let found = || found.iter().flat_map(Covered::runs).filter(|range| !inside(range));
    rewrite(text, |mark| merged(found(), &edits, mark))
}

/// Calls `mark` with each range of a text that the matches `found` (apart and in order, none touching) and the password
/// fields' `edits` (apart and in order) leave as one mark, in order, with what replaces it: the ranges as
/// [`password_edits`] and [`covered`] give them, merged in one pass, and none of them copied.
fn merged(
    found: impl Iterator<Item = (usize, usize)>,
    edits: &[(usize, usize, String, bool)],
    mut mark: impl FnMut(usize, usize, &str),
) {
    // In order of their start, the longer first, a match before a password field's value on the same range.
    let mut found = found.peekable();
    let mut edits = edits.iter().peekable();
    let mut next = || match (found.peek(), edits.peek()) {
        (Some(&(s, e)), Some(edit)) if (s, std::cmp::Reverse(e)) <= (edit.0, std::cmp::Reverse(edit.1)) => {
            found.next().map(|(s, e)| (s, e, None, false))
        }
        (Some(_), None) => found.next().map(|(s, e)| (s, e, None, false)),
        _ => edits.next().map(|(s, e, with, whole)| (*s, *e, Some(with.as_str()), *whole)),
    };
    let mut last: Option<(usize, usize, Option<&str>, bool)> = None;
    while let Some((start, end, with, whole)) = next() {
        match last.as_mut() {
            // Wholly inside a mark or a password field's value already taken: a recorded match inside the value (the
            // person typed it there) leaves the value's own replacement, so JSON text stays JSON. Inside a string
            // scrubbed on its own, it reaches the string's quotes, and hides the whole string.
            Some(last) if end <= last.1 && !last.3 => {}
            Some(last) if start < last.1 || (start == last.1 && with.is_none() && last.2.is_none()) => {
                last.1 = last.1.max(end);
                (last.2, last.3) = (None, false);
            }
            _ => {
                if let Some((start, end, with, _)) = last.replace((start, end, with, whole)) {
                    mark(start, end, with.unwrap_or(MARK));
                }
            }
        }
    }
    if let Some((start, end, with, _)) = last {
        mark(start, end, with.unwrap_or(MARK));
    }
}

/// `text` with the ranges `marks` gives (apart and in order, each with what replaces it) replaced, or none if it gives
/// none. They are given twice, so the copy is made at its length once: a buffer that doubles as it grows may take
/// twice that.
fn rewrite(text: &str, marks: impl Fn(&mut dyn FnMut(usize, usize, &str))) -> Option<String> {
    let (mut any, mut length) = (false, text.len());
    marks(&mut |start, end, with| {
        any = true;
        length = length - (end - start) + with.len();
    });
    if !any {
        return None;
    }
    let mut out = String::with_capacity(length);
    let mut at = 0;
    marks(&mut |start, end, with| {
        out.push_str(&text[at..start]);
        out.push_str(with);
        at = end;
    });
    out.push_str(&text[at..]);
    Some(out)
}

/// The password fields' values in `text`: byte ranges of it, apart and in order, what replaces each, and whether the
/// range is a whole JSON string scrubbed on its own (recorded strings included). Text the walk cannot follow (not JSON,
/// JSON after a prefix or beside more text, or nested deeper than [`MAX_DEPTH`]) loses the rest of a tree row after
/// such a role, as the driver's Markdown writes one, and everything after a role written as a JSON string, whose value
/// may be on any later line.
fn password_edits(
    matcher: Option<&Matcher>,
    text: &str,
    depth: usize,
    levels: usize,
) -> Vec<(usize, usize, String, bool)> {
    if !text.contains(PASSWORD) {
        return Vec::new();
    }
    if text.trim_start().starts_with(['{', '[', '"']) {
        let mut json = Json { text, matcher, levels, at: 0, edits: Vec::new() };
        if json.value(depth).is_some() && json.end() {
            // On the same range a password field's value goes before its string's own scrub.
            json.edits.sort_by_key(|&(start, end, _, whole)| (start, std::cmp::Reverse(end), whole));
            let mut at = 0;
            json.edits.retain(|&(start, end, ..)| (start >= at).then(|| at = end).is_some());
            return json.edits;
        }
    }
    let mut edits = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if let Some(at) = content.find(PASSWORD) {
            let (from, json) = (offset + at + PASSWORD.len(), content[..at].ends_with('"'));
            let to = if json { text.len() } else { offset + content.len() };
            if from < to {
                edits.push((from, to, format!(" \"{MARK}\""), false));
            }
            if json {
                break;
            }
        }
        offset += line.len();
    }
    edits
}

/// A walk over one JSON text that collects the edits [`password_edits`] makes to it, as byte ranges and what replaces
/// them, without decoding the text into values.
struct Json<'a> {
    text: &'a str,
    matcher: Option<&'a Matcher>,
    /// How many times the text was decoded already (see [`scrub_text`]).
    levels: usize,
    at: usize,
    edits: Vec<(usize, usize, String, bool)>,
}

impl Json<'_> {
    fn byte(&mut self) -> Option<u8> {
        let bytes = self.text.as_bytes();
        while matches!(bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
        bytes.get(self.at).copied()
    }

    fn end(&mut self) -> bool {
        self.byte().is_none()
    }

    /// One value: its range, and whether it is a string.
    fn value(&mut self, depth: usize) -> Option<(usize, usize, bool)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let first = self.byte()?;
        let start = self.at;
        match first {
            b'{' => self.object(depth)?,
            b'[' => self.array(depth)?,
            b'"' => {
                self.string()?;
                let raw = &self.text[start..self.at];
                if raw.contains(PASSWORD)
                    && let Some(encoded) = self.inner(raw, depth)?
                {
                    self.edits.push((start, self.at, encoded, true));
                }
                return Some((start, self.at, true));
            }
            _ => self.scalar()?,
        }
        Some((start, self.at, false))
    }

    /// A string of the text that holds a password field's role, `raw` as written: scrubbed as a text of its own and
    /// encoded again, or none if that changes nothing (and none at all if it is not a string). Decoded [`MAX_LEVELS`]
    /// times already, it is hidden whole instead: what a string escaped more often holds is not followed.
    fn inner(&self, raw: &str, depth: usize) -> Option<Option<String>> {
        if self.levels == MAX_LEVELS {
            return Some(Some(format!("\"{MARK}\"")));
        }
        // Each copy is dropped once the next is made: a string may be most of the observation.
        let decoded = serde_json::from_str::<String>(raw).ok()?;
        let Some(hidden) = scrub_text(self.matcher, &decoded, depth + 1, self.levels + 1) else { return Some(None) };
        drop(decoded);
        let encoded = serde_json::to_string(&hidden).expect("a string is JSON");
        drop(hidden);
        // Encoded again, the string is looked through once more as written, in one scan: a recorded value may be one
        // that only its escaped form holds, while its decoded forms are what its own scrub just looked through.
        let Some(found) = self.matcher.map(|matcher| covered(matcher, &encoded, 0)) else { return Some(Some(encoded)) };
        let marked = rewrite(&encoded, |mark| found.runs().for_each(|(s, e)| mark(s, e, MARK)));
        Some(Some(marked.unwrap_or(encoded)))
    }

    fn string(&mut self) -> Option<()> {
        let bytes = self.text.as_bytes();
        let mut at = self.at + 1;
        loop {
            match *bytes.get(at)? {
                b'"' => break,
                b'\\' => at += 2,
                byte if byte < 0x20 => return None,
                _ => at += 1,
            }
        }
        self.at = at + 1;
        Some(())
    }

    /// `true`, `false`, `null` or a number.
    fn scalar(&mut self) -> Option<()> {
        let bytes = self.text.as_bytes();
        let start = self.at;
        while bytes.get(self.at).is_some_and(|b| !matches!(b, b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
        let token = &self.text[start..self.at];
        (matches!(token, "true" | "false" | "null") || token.parse::<serde_json::Number>().is_ok()).then_some(())
    }

    fn array(&mut self, depth: usize) -> Option<()> {
        self.at += 1;
        if self.byte()? == b']' {
            self.at += 1;
            return Some(());
        }
        loop {
            self.value(depth + 1)?;
            match self.byte()? {
                b',' => self.at += 1,
                b']' => break,
                _ => return None,
            }
        }
        self.at += 1;
        Some(())
    }

    /// An object; a password field's (its `role` the role of one, as the driver writes it) loses its `value` and
    /// `text`.
    fn object(&mut self, depth: usize) -> Option<()> {
        self.at += 1;
        let (mut password, mut fields) = (false, Vec::new());
        if self.byte()? == b'}' {
            self.at += 1;
            return Some(());
        }
        loop {
            if self.byte()? != b'"' {
                return None;
            }
            let key = self.at;
            self.string()?;
            let key = &self.text[key..self.at];
            if self.byte()? != b':' {
                return None;
            }
            self.at += 1;
            let (start, end, _) = self.value(depth + 1)?;
            match key {
                "\"role\"" => password |= self.text[start..end] == format!("\"{PASSWORD}\""),
                "\"value\"" | "\"text\"" => fields.push((start, end)),
                _ => {}
            }
            match self.byte()? {
                b',' => self.at += 1,
                b'}' => break,
                _ => return None,
            }
        }
        self.at += 1;
        if password {
            self.edits.extend(fields.into_iter().map(|(start, end)| (start, end, format!("\"{MARK}\""), false)));
        }
        Some(())
    }
}

/// Text typed into a field, as the field holds it after the edits followed, and where in it (a byte offset, on a
/// character boundary) the field's cursor stands.
#[derive(Default, Debug)]
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
    fn a_password_field_in_json_text_loses_its_value_and_the_text_stays_json() {
        let mut r = Redactions::default();
        r.typed("alice-secret");
        r.boundary();
        let state = json!({"elements": [
            {"role": "password text", "label": "Password", "value": "autofilled-pass"},
            {"role": "entry", "label": "User", "value": "alice-secret"},
            {"role": "push button", "label": "Sign in"},
        ], "tree_markdown": "- password text \"hunter2\"\n- push button \"Sign in\""});
        // Whether or not a string was recorded.
        for (r, typed) in [(Redactions::default(), "alice-secret"), (r, "[redacted]")] {
            let mut result = json!({"content": [{"type": "text", "text": state.to_string()}]});
            r.apply(&mut result);
            let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(text["elements"][0]["value"], "[redacted]");
            assert_eq!(text["elements"][2]["label"], "Sign in");
            assert_eq!(text["tree_markdown"], "- password text \"[redacted]\"\n- push button \"Sign in\"");
            assert_eq!(text["elements"][1]["value"], typed);
        }
    }

    #[test]
    fn a_typed_password_in_a_password_field_s_value_leaves_the_text_json() {
        let mut r = Redactions::default();
        r.typed("hunter2-pass");
        r.boundary();
        let state = json!({"elements": [{"role": "password text", "label": "Password", "value": "hunter2-pass"}]});
        let mut observation = json!(state.to_string());
        r.apply(&mut observation);
        let text: Value = serde_json::from_str(observation.as_str().unwrap()).unwrap();
        assert_eq!(text["elements"][0], json!({"role": "password text", "label": "Password", "value": "[redacted]"}));
    }

    #[test]
    fn a_recorded_value_from_a_string_s_quote_hides_the_whole_string_its_own_scrub_rewrote() {
        let mut r = Redactions::default();
        r.keep("\"hunter2 password text suffix");
        let mut observation = json!(r#"["hunter2 password text suffix"]"#);
        r.apply(&mut observation);
        assert_eq!(observation, "[[redacted]]");
    }

    #[test]
    fn a_64_kib_multi_line_paste_with_quotes_leaves_room_in_the_set() {
        // A pretty-printed JSON document of nearly 64 KiB, pasted once, its lines ending in line feeds or in carriage
        // returns and line feeds.
        for (lines, end) in [(2250, "\n"), (2184, "\r\n")] {
            let paste = (0..lines).map(|n| format!("  \"key{n:04}\": \"value-012345\",")).collect::<Vec<_>>().join(end);
            assert!(paste.len() <= 64 * 1024 && paste.len() > 63 * 1024, "{}", paste.len());
            let mut r = Redactions::default();
            r.pasted(&paste);
            r.boundary();
            r.typed("hunter2-wifi");
            r.boundary();
            assert!(!r.saturated, "{}", r.bytes);
            // A script returns the field's value, in its text item as JSON.
            let value = paste.replace("\r\n", "\n");
            assert!(MAX_BYTES - r.bytes > 8 * 1024, "{}", r.bytes);
            let mut result = json!({"content": [{"type": "text", "text": json!({"value": value}).to_string()}]});
            r.apply(&mut result);
            assert_eq!(result["content"][0]["text"], "{\"value\":\"[redacted]\"}");
        }
    }

    #[test]
    fn a_value_escaped_in_any_text_is_hidden_and_the_rest_stays_as_written() {
        let mut r = Redactions::default();
        for typed in ["Tr0ub\"4dor", "1234", "caf\u{e9}-\u{1f600}-key"] {
            r.typed(typed);
            r.boundary();
        }
        let nested = |text: String| (0..200).fold(text, |text, _| format!("[{text}]"));
        let cases = [
            // After a prefix, as an error quotes JSON.
            (r#"error: {"value":"Tr0ub\"4dor"}"#.to_owned(), r#"error: {"value":"[redacted]"}"#.to_owned()),
            // Nested deeper than any parser follows.
            (nested(r#""Tr0ub\"4dor""#.into()), nested(r#""[redacted]""#.into())),
            // JSON text inside JSON text: escaped twice.
            (json!(json!(["Tr0ub\"4dor"]).to_string()).to_string(), r#""[\"[redacted]\"]""#.to_owned()),
            // Beside a number that is a recorded string itself, and a big one kept exactly as written.
            (
                r#"{ "pin": 1234, "text": "pa Tr0ub\"4dor", "id": 18446744073709551617.50 }"#.to_owned(),
                r#"{ "pin": [redacted], "text": "pa [redacted]", "id": 18446744073709551617.50 }"#.to_owned(),
            ),
            // Every character written as \u, a surrogate pair included.
            (r#""café-😀-key""#.to_owned(), r#""[redacted]""#.to_owned()),
        ];
        for (text, expected) in cases {
            let mut observation = json!(text);
            r.apply(&mut observation);
            assert_eq!(observation, expected, "{text}");
        }
        // A backslash that starts no escape is a character of its own.
        let mut observation = json!(r#"x\qTr0ub\\\"4dor\u12"#);
        r.apply(&mut observation);
        assert_eq!(observation, r#"x\q[redacted]\u12"#);
    }

    #[test]
    fn a_match_that_starts_or_ends_inside_an_escape_hides_all_of_it_and_the_text_stays_json() {
        // A piece of a run whose cursor was lost, recorded, that starts on an escape's letter (`nabc` in `x\nabc`, which
        // decoded holds no `nabc`), on its second backslash, or on its hex digits, or ends inside a surrogate pair.
        let cases = [
            ("nabc", r#"{"t":"x\nabc"}"#, "x[redacted]"),
            ("tabc", r#"{"t":"x\tabcd"}"#, "x[redacted]d"),
            (r"\bob", r#"{"t":"C:\\bob"}"#, "C:[redacted]"),
            ("0078y", r#"{"t":"abc\u0078y"}"#, "abc[redacted]"),
            ("de00!", r#"{"t":"q\ud83d\ude00!"}"#, "q[redacted]"),
        ];
        for (kept, text, expected) in cases {
            let mut r = Redactions::default();
            r.keep(kept);
            // As JSON text, and as JSON text inside JSON text (a script's result), where the escapes are escaped again,
            // up to the levels decoded, the last of which still holds escapes it does not decode.
            let wrapped = (0..MAX_LEVELS).scan(text.to_owned(), |text, _| {
                *text = json!({"result": text}).to_string();
                Some(text.clone())
            });
            for text in [text.to_owned()].into_iter().chain(wrapped) {
                let mut observation = json!({"content": [{"type": "text", "text": text}]});
                r.apply(&mut observation);
                let mut seen = observation["content"][0]["text"].as_str().unwrap().to_owned();
                let inner = loop {
                    let value: Value = serde_json::from_str(&seen).unwrap_or_else(|e| panic!("{kept} in {text}: {e}"));
                    match value.get("result") {
                        Some(result) => seen = result.as_str().unwrap().to_owned(),
                        None => break value,
                    }
                };
                assert_eq!(inner["t"], expected, "{kept} in {text}");
            }
        }
    }

    #[test]
    fn a_recorded_value_holding_the_password_role_stays_hidden_whole() {
        for typed in ["hunter2-password text-suffix", "[hunter2-password text-suffix]"] {
            let mut r = Redactions::default();
            r.typed(typed);
            r.boundary();
            let mut observation = json!([typed, format!(r#"{{"role":"entry","value":"{typed}"}}"#)]);
            r.apply(&mut observation);
            let seen = observation.to_string();
            assert!(!seen.contains("hunter2") && !seen.contains("suffix"), "{seen}");
        }
        // A password row of the diff format loses its value too.
        let mut observation = json!(["~ password text [0] \"autofilled-pass\"", "+ password text \"autofilled-pass\""]);
        Redactions::default().apply(&mut observation);
        assert!(!observation.to_string().contains("autofilled"), "{observation}");
    }

    #[test]
    fn a_password_value_holding_the_role_and_a_recorded_value_at_a_quote_stay_hidden() {
        let mut r = Redactions::default();
        r.typed("user");
        r.boundary();
        let mut observation = json!(r#"{"role":"password text","value":"autofilled-user-password text"}"#);
        r.apply(&mut observation);
        assert!(!observation.to_string().contains("autofilled"), "{observation}");
        let mut r = Redactions::default();
        r.typed("\"hunter2");
        r.boundary();
        let mut observation = json!(r#""hunter2 password text suffix""#);
        r.apply(&mut observation);
        assert!(!observation.to_string().contains("hunter2"), "{observation}");
    }

    #[test]
    fn a_recorded_value_only_an_escaped_form_holds_stays_hidden_in_a_scrubbed_string() {
        let mut r = Redactions::default();
        r.keep(r#"\"hunter2"#);
        let mut observation = json!({"result": r#""prefix\"hunter2 password text suffix""#});
        r.apply(&mut observation);
        assert!(!observation.to_string().contains("hunter2"), "{observation}");
    }

    #[test]
    fn a_password_field_is_hidden_before_recorded_names_change_the_text() {
        let mut r = Redactions::default();
        r.typed("value");
        r.boundary();
        let mut observation = json!(r#"{"role":"password text","value":"autofilled-pass"}"#);
        r.apply(&mut observation);
        assert_eq!(observation, r#"{"role":"password text","[redacted]":"[redacted]"}"#);
        // Inside JSON text inside JSON text, and nested deeper than the walk follows, where its line loses the rest.
        let inner = json!({"elements": [{"role": "password text", "value": "autofilled-pass"}, {"role": "entry"}]});
        let mut observation = json!(json!([inner.to_string()]).to_string());
        r.apply(&mut observation);
        let outer: Vec<String> = serde_json::from_str(observation.as_str().unwrap()).unwrap();
        let inner: Value = serde_json::from_str(&outer[0]).unwrap();
        assert_eq!(
            inner["elements"][0],
            json!({"role": "password text", "[redacted]": "[redacted]"}),
            "the name recorded too"
        );
        assert_eq!(inner["elements"][1]["role"], "entry");
        let deep =
            (0..200).fold(r#"{"role":"password text","text":"autofilled-pass"}"#.to_owned(), |t, _| format!("[{t}]"));
        let mut observation = json!(deep);
        r.apply(&mut observation);
        assert!(!observation.to_string().contains("autofilled"), "{observation}");
    }

    #[test]
    fn a_string_holding_the_password_role_past_the_levels_decoded_is_hidden_whole() {
        let mut r = Redactions::default();
        r.typed("hunter2-zzzz");
        r.boundary();
        let inner =
            json!({"elements": [{"role": "password text", "value": "autofilled-pass"}], "bulk": "x".repeat(1 << 16)});
        let wrapped = (0..10).fold(inner.to_string(), |text, _| json!([text, "password text"]).to_string());
        let mut observation = json!(wrapped);
        r.apply(&mut observation);
        // Each level as a JSON string again, down to the last one decoded, whose strings are hidden whole.
        let mut text = observation.as_str().unwrap().to_owned();
        for _ in 0..MAX_LEVELS {
            let level: Vec<String> = serde_json::from_str(&text).unwrap();
            assert_eq!(level[1], "password text");
            text = level[0].clone();
        }
        assert_eq!(text, r#"["[redacted]","[redacted]"]"#);
    }

    #[test]
    fn a_password_field_in_json_text_the_walk_cannot_follow_loses_every_later_line() {
        let doc =
            serde_json::to_string_pretty(&json!({"elements": [{"role": "password text", "value": "autofilled-pass"}]}));
        let doc = doc.unwrap();
        for text in [format!("error: {doc}"), format!("{doc}\n{doc}")] {
            let mut observation = json!(text);
            Redactions::default().apply(&mut observation);
            let seen = observation.as_str().unwrap();
            assert!(!seen.contains("autofilled") && seen.contains("[redacted]"), "{seen}");
        }
        // A Markdown tree loses only the rest of the row.
        let mut observation = json!("- password text \"autofilled-pass\"\n- push button \"Sign in\"");
        Redactions::default().apply(&mut observation);
        assert_eq!(observation, "- password text \"[redacted]\"\n- push button \"Sign in\"");
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
        let ranges = covered(&matcher, &text, MAX_LEVELS);
        assert_eq!(ranges.runs().collect::<Vec<_>>(), [(0, text.len())]);
        assert_eq!(ranges.words.capacity(), text.len() / 64, "the ranges held a bit per byte of the text");
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
            let ranges: Vec<_> = covered(&built(&r), &text, MAX_LEVELS).runs().collect();
            assert_eq!(ranges, expected, "{:?} in {text}", r.entries);
        }
    }

    #[test]
    fn ranges_that_only_touch_merge_and_apart_ones_stay_apart() {
        let mut r = Redactions::default();
        r.keep("abcd");
        r.keep("efgh");
        let ranges: Vec<_> = covered(&built(&r), "abcdefgh abcd", MAX_LEVELS).runs().collect();
        assert_eq!(ranges, [(0, 8), (9, 13)]);
    }

    /// What [`covered`] gave before its matches were kept as bits: every range kept as found, the levels merged by
    /// sorting, then widened over every escape of every level scanned that one ends inside, until none does.
    fn listed(matcher: &Matcher, text: &str, most: usize) -> Vec<(usize, usize)> {
        let mut escapes: Vec<(usize, usize)> = Vec::new();
        let mut listed_in = |bytes: &mut dyn Iterator<Item = Byte>| {
            let mut ranges: Vec<(usize, usize)> = Vec::new();
            let size = matcher.widest.min(text.len()).max(1);
            let (mut starts, mut state) = (vec![0; size], 0);
            for (at, (byte, from, to)) in bytes.enumerate() {
                if to - from > 1 {
                    escapes.push((from, to));
                }
                starts[at % size] = from;
                state = matcher.step(state, byte);
                let found = matcher.longest[state] as usize;
                if found == 0 {
                    continue;
                }
                let mut start = starts[(at + 1 - found) % size];
                while let Some(&(s, e)) = ranges.last()
                    && e >= start
                {
                    start = start.min(s);
                    ranges.pop();
                }
                ranges.push((start, to));
            }
            ranges
        };
        let mut ranges = listed_in(&mut written(text));
        let (mut levels, mut deeper) = (0, text.contains('\\'));
        let merge = |ranges: Vec<(usize, usize)>| {
            ranges.into_iter().fold(Vec::new(), |mut merged: Vec<(usize, usize)>, (start, end)| {
                match merged.last_mut() {
                    Some(last) if start <= last.1 => last.1 = last.1.max(end),
                    _ => merged.push((start, end)),
                }
                merged
            })
        };
        while deeper && levels < most {
            levels += 1;
            let (escaped, backslash) = (Cell::new(false), Cell::new(false));
            let mut bytes: Box<dyn Iterator<Item = Byte> + '_> = Box::new(written(text));
            for level in 1..=levels {
                bytes = Box::new(Unescaped::new(bytes, (level == levels).then_some(&escaped)));
            }
            let mut bytes = bytes.inspect(|&(byte, ..)| backslash.set(backslash.get() || byte == b'\\'));
            ranges.extend(listed_in(&mut bytes));
            deeper = escaped.get() && backslash.get();
            ranges.sort_unstable();
            ranges = merge(ranges);
        }
        if deeper {
            let mut bytes: Box<dyn Iterator<Item = Byte> + '_> = Box::new(written(text));
            for _ in 0..=levels {
                bytes = Box::new(Unescaped::new(bytes, None));
            }
            escapes.extend(bytes.map(|(_, from, to)| (from, to)).filter(|(from, to)| to - from > 1));
        }
        loop {
            let mut widened: Vec<(usize, usize)> = ranges
                .iter()
                .map(|&(mut start, mut end)| {
                    for &(from, to) in &escapes {
                        if from < start && start < to {
                            start = from;
                        }
                        if from < end && end < to {
                            end = to;
                        }
                    }
                    (start, end)
                })
                .collect();
            widened.sort_unstable();
            let widened = merge(widened);
            if widened == ranges {
                return ranges;
            }
            ranges = widened;
        }
    }

    /// What [`scrub_text`] gave before it merged its ranges in one pass: every range copied, sorted and merged into a
    /// list, then applied.
    fn listed_scrub(matcher: Option<&Matcher>, text: &str) -> Option<String> {
        let mut ranges: Vec<(usize, usize, Option<String>, bool)> = Vec::new();
        let edits = password_edits(matcher, text, 0, 0);
        if let Some(matcher) = matcher {
            let wholes: Vec<(usize, usize)> = edits.iter().filter(|e| e.3).map(|&(s, e, ..)| (s, e)).collect();
            let inside = |start: usize, end: usize| {
                let at = wholes.partition_point(|&(s, _)| s < start);
                at > 0 && wholes[at - 1].0 < start && end < wholes[at - 1].1
            };
            let found = listed(matcher, text, MAX_LEVELS);
            ranges.extend(found.into_iter().filter(|&(s, e)| !inside(s, e)).map(|(s, e)| (s, e, None, false)));
        }
        ranges.extend(edits.into_iter().map(|(start, end, with, whole)| (start, end, Some(with), whole)));
        if ranges.is_empty() {
            return None;
        }
        ranges.sort_by_key(|&(start, end, ..)| (start, std::cmp::Reverse(end)));
        let mut merged: Vec<(usize, usize, Option<String>, bool)> = Vec::new();
        for (start, end, with, whole) in ranges {
            match merged.last_mut() {
                Some(last) if end <= last.1 && !last.3 => {}
                Some(last) if start < last.1 || (start == last.1 && with.is_none() && last.2.is_none()) => {
                    last.1 = last.1.max(end);
                    (last.2, last.3) = (None, false);
                }
                _ => merged.push((start, end, with, whole)),
            }
        }
        let mut out = String::new();
        let mut at = 0;
        for (start, end, with, _) in merged {
            out.push_str(&text[at..start]);
            out.push_str(with.as_deref().unwrap_or(MARK));
            at = end;
        }
        out.push_str(&text[at..]);
        Some(out)
    }

    #[test]
    fn the_scrub_gives_what_it_gave_when_it_listed_every_range() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as usize % n
        };
        // Pieces that make recorded values, password fields in both formats, escapes and the quotes around them.
        let pieces = [
            "abcd",
            "cdef",
            "abcdefgh",
            "ab",
            "ef",
            " ",
            "x",
            "\n",
            "\"",
            "\\\"",
            "\\\\",
            "\\n",
            "\\\\n",
            "\\u0061",
            "pa\\\"ss",
            "password text",
            "\"password text\"",
            "\"role\":\"password text\",\"value\":\"",
            "\"text\":\"",
            "\",",
            "{\"role\":\"password text\",\"value\":\"abcd\"}",
            "[\"abcd\",",
            "]",
            "{",
            "}",
            ":",
            ",",
            "password text abcd\n",
        ];
        // Some match a password field's value or a whole string exactly, as written, and one an escape's letter.
        let entries = [
            "abcd",
            "cdef",
            "abcdefgh",
            "d ab",
            "pa\"ss",
            "ab\ncd",
            "efab",
            "abcdabcdabcd",
            "\"abcd\"",
            " abcd",
            "\"password text\"",
            "nabc",
        ];
        for round in 0..3000 {
            let mut r = Redactions::default();
            for _ in 0..1 + next(3) {
                r.keep(entries[next(entries.len())]);
            }
            let matcher = built(&r);
            let text: String = (0..next(40)).map(|_| pieces[next(pieces.len())]).collect();
            // As written, and as a JSON string or a JSON value holding it, which the walk follows.
            let json = json!({"role": "password text", "value": text, "text": [text, {"abcd": text}]}).to_string();
            for text in [text.clone(), serde_json::to_string(&text).unwrap(), json] {
                let expected = listed_scrub(Some(&matcher), &text);
                assert_eq!(
                    scrub_text(Some(&matcher), &text, 0, 0),
                    expected,
                    "round {round}: {:?} in {text}",
                    r.entries
                );
            }
        }
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
        // Kept whole, never piece by piece.
        assert!(!r.saturated && r.bytes < paste.len() + MAX_RUN, "{}", r.bytes);
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
        assert!(!r.scrub().build(), "nothing of an observation is searched any more");
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
