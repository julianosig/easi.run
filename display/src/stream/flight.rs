//! What one viewer was sent and has not acknowledged yet, and whether its encoder may capture another frame.
//!
//! The client acknowledges every `Video` and `Tiles` message as it arrives (`MediaAck`, cumulative by revision). From the
//! acknowledgements come the connection's round trip (the shortest of the last [`RTT_WINDOW`]) and its delivery rate
//! (the delivery-rate sample of BBR: what was acknowledged since a message was sent, over the longer of the time those
//! acknowledgements took to come and the time the messages before it took to be sent; the highest of the last
//! [`RATE_WINDOW`]). The encoder captures only while what is in flight is younger than the round trip plus
//! [`MEDIA_AGE_ALLOWANCE_MS`] and smaller than what the connection delivers in that time, so a slow path, or a client
//! that stopped reading, gets fewer and current frames instead of a backlog that waits in buffers downstream (the admin's
//! socket, a proxy, an SSH tunnel), where nothing can drop a stale frame.
//!
//! What goes out is also paced: every byte sent (media and pointer images) is charged to a bucket that refills at
//! [`PACE_GAIN`] times the delivery rate, never slower than [`PACE_FLOOR`], and holds at most [`PACE_DEPTH`]; nothing
//! more is sent while it is in debt. A large update leaves as a run of small bursts instead of one burst at the sender's
//! line rate, which a shallow queue on a long path would mostly drop. The floor is there because the delivery rate a
//! stream measures is mostly its own: a video at 2.5 Mbit/s never shows the path could carry more, and pacing its
//! keyframes at twice that would hold every frame behind them.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use display_protocol::MEDIA_AGE_ALLOWANCE_MS;

/// The least media in flight, whatever the estimates say: a video keyframe and a refresh's tiles beside it, which a
/// stream that mostly sends small frames measures no room for.
pub const MIN_WINDOW: usize = 256 * 1024;
/// The most media in flight: about 39 Mbit/s at a 215 ms round trip.
pub const MAX_WINDOW: usize = 1024 * 1024;
/// How long the shortest round trip measured stands, and how far back the highest delivery rate is taken.
const RTT_WINDOW: Duration = Duration::from_secs(10);
const RATE_WINDOW: Duration = Duration::from_secs(10);
const ALLOWANCE: Duration = Duration::from_millis(MEDIA_AGE_ALLOWANCE_MS);
/// Delivery-rate and round-trip samples kept for the windows' maximum and minimum, at most.
const MAX_RATE_SAMPLES: usize = 1024;
/// How much faster than the measured delivery rate media may leave: room for the rate to grow (each round trip's
/// samples measure what was sent), as TCP's slow start doubles.
pub const PACE_GAIN: f64 = 2.0;
/// The slowest pacing, in bytes per second (25 Mbit/s): well above a video's bitrate, and spread enough that a refresh
/// of a few hundred KB crosses a long path's shallow queue without overflowing it.
pub const PACE_FLOOR: f64 = 3_125_000.0;
/// The most the pacing bucket holds: the largest burst after a pause.
pub const PACE_DEPTH: f64 = 64.0 * 1024.0;

/// One media message in flight.
struct Sent {
    revision: u64,
    at: Instant,
    bytes: usize,
    /// What had been acknowledged when it was sent, when the last of that came, and when the message acknowledged
    /// last had been sent.
    delivered: u64,
    delivered_at: Instant,
    first_at: Instant,
}

#[derive(Default)]
pub struct Flight {
    sent: VecDeque<Sent>,
    bytes: usize,
    /// The newest revision sent.
    newest: u64,
    /// Bytes acknowledged so far, when the last acknowledgement came, and when the message it acknowledged was sent.
    delivered: u64,
    delivered_at: Option<Instant>,
    first_at: Option<Instant>,
    /// Round-trip samples of the window, each longer than the ones before it: the first is the window's shortest, and
    /// when it leaves the window the next shortest stands. The latest sample is always kept.
    rtts: VecDeque<(Instant, Duration)>,
    /// Delivery-rate samples (bytes per second) of the window, each lower than the ones before it: the first is the
    /// window's highest. The latest sample stands in once the window holds none.
    rates: VecDeque<(Instant, f64)>,
    latest_rate: Option<f64>,
    /// What is in flight outlived the allowance; cleared by the next acknowledgement.
    late: bool,
    /// The pacing bucket, in bytes (negative while in debt), as of `paced_at`.
    tokens: f64,
    paced_at: Option<Instant>,
}

/// An acknowledgement of a revision past the newest sent.
#[derive(Debug, PartialEq, Eq)]
pub struct Unsent;

impl Flight {
    /// Records a media message of `revision` and `bytes` (as queued for the admin) sent at `now`, before it is written,
    /// so its acknowledgement cannot come first.
    pub fn sent(&mut self, revision: u64, bytes: usize, now: Instant) {
        debug_assert!(revision >= self.newest, "media revisions never decrease");
        if self.sent.is_empty() {
            // Nothing was in flight: the delivery rate is measured from now, not from the last acknowledgement.
            self.first_at = Some(now);
            self.delivered_at = Some(now);
        }
        self.sent.push_back(Sent {
            revision,
            at: now,
            bytes,
            delivered: self.delivered,
            delivered_at: self.delivered_at.unwrap_or(now),
            first_at: self.first_at.unwrap_or(now),
        });
        self.newest = revision;
        self.bytes += bytes;
    }

    /// The client received every media message up to `revision` at `now`; refused for a revision past the newest sent.
    pub fn acked(&mut self, revision: u64, now: Instant) -> Result<(), Unsent> {
        if revision > self.newest {
            return Err(Unsent);
        }
        let mut newest = None;
        while self.sent.front().is_some_and(|s| s.revision <= revision) {
            let sent = self.sent.pop_front().expect("checked above");
            self.bytes -= sent.bytes;
            self.delivered += sent.bytes as u64;
            newest = Some(sent);
        }
        let Some(sent) = newest else { return Ok(()) };
        let rtt = now.saturating_duration_since(sent.at);
        push_extremum(&mut self.rtts, now, rtt, RTT_WINDOW, |old, new| old >= new);
        let elapsed =
            now.saturating_duration_since(sent.delivered_at).max(sent.at.saturating_duration_since(sent.first_at));
        if elapsed >= Duration::from_millis(1) {
            self.rate((self.delivered - sent.delivered) as f64 / elapsed.as_secs_f64(), now);
        }
        self.delivered_at = Some(now);
        self.first_at = Some(sent.at);
        self.late = false;
        Ok(())
    }

    /// The media message of `revision` will not be acknowledged (the admin's network simulator lost it): it leaves
    /// the flight and says nothing about the connection.
    pub fn discarded(&mut self, revision: u64) {
        if let Some(i) = self.sent.iter().position(|s| s.revision == revision) {
            let sent = self.sent.remove(i).expect("found above");
            self.bytes -= sent.bytes;
        }
    }

    /// The viewer's media changed paths (see `Link::PathChanged`): nothing in flight will be acknowledged, and what
    /// was measured of the old path no longer holds. Only the newest revision sent stays, so an acknowledgement of a
    /// revision past the newest sent is still refused.
    pub fn path_changed(&mut self) {
        *self = Flight { newest: self.newest, ..Flight::default() };
    }

    fn rate(&mut self, rate: f64, now: Instant) {
        push_extremum(&mut self.rates, now, rate, RATE_WINDOW, |old, new| old <= new);
        self.latest_rate = Some(rate);
    }

    /// How old media in flight may get at `now`: the round trip (the shortest measured within the window, else the
    /// latest measured; none before one is measured) plus the allowance.
    pub fn horizon(&self, now: Instant) -> Duration {
        let recent = self.rtts.iter().find(|(at, _)| now.saturating_duration_since(*at) <= RTT_WINDOW);
        recent.or(self.rtts.back()).map_or(Duration::ZERO, |(_, rtt)| *rtt) + ALLOWANCE
    }

    /// The delivery rate in bytes per second: the window's highest, else the latest measured, else `seed` (the
    /// bitrate setting's).
    pub fn delivery_rate(&self, now: Instant, seed: f64) -> f64 {
        let recent = self.rates.iter().find(|(at, _)| now.saturating_duration_since(*at) <= RATE_WINDOW);
        recent.map(|(_, r)| *r).or(self.latest_rate).unwrap_or(seed)
    }

    /// What may be in flight: what the connection delivers over the horizon, from [`MIN_WINDOW`] to [`MAX_WINDOW`].
    pub fn window(&self, now: Instant, seed: f64) -> usize {
        let window = self.delivery_rate(now, seed) * self.horizon(now).as_secs_f64();
        (window as usize).clamp(MIN_WINDOW, MAX_WINDOW)
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// How long the oldest media in flight has been, at `now` (zero for none).
    pub fn oldest_age(&self, now: Instant) -> Duration {
        self.sent.front().map_or(Duration::ZERO, |s| now.saturating_duration_since(s.at))
    }

    /// How fast the pacing bucket refills at `now`, in bytes per second.
    fn pace(&self, now: Instant, seed: f64) -> f64 {
        (PACE_GAIN * self.delivery_rate(now, seed)).max(PACE_FLOOR)
    }

    /// Refills the pacing bucket up to `now`.
    fn refill(&mut self, now: Instant, seed: f64) {
        let rate = self.pace(now, seed);
        let elapsed = self.paced_at.map_or(f64::INFINITY, |at| now.saturating_duration_since(at).as_secs_f64());
        self.tokens = (self.tokens + rate * elapsed).min(PACE_DEPTH);
        self.paced_at = Some(now);
    }

    /// Charges `bytes` sent at `now` to the pacing bucket.
    pub fn charge(&mut self, bytes: usize, now: Instant, seed: f64) {
        self.refill(now, seed);
        self.tokens -= bytes as f64;
    }

    /// How long until the pacing bucket is out of debt at `now`; zero when it is.
    pub fn pacing(&mut self, now: Instant, seed: f64) -> Duration {
        self.refill(now, seed);
        // A byte's rounding is no debt.
        if self.tokens > -1.0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(-self.tokens / self.pace(now, seed))
    }

    /// The bytes the pacing bucket holds at `now` (none while in debt): what an encode may send at once.
    pub fn credit(&mut self, now: Instant, seed: f64) -> usize {
        self.refill(now, seed);
        self.tokens.max(0.0) as usize
    }

    /// Whether the encoder may capture another frame at `now`: nothing in flight is older than the horizon, and less
    /// than the window is. Once what is in flight outlives the horizon, the delivery rate measured before no longer
    /// describes the path, and only samples from the acknowledgements still to come count.
    pub fn admits(&mut self, now: Instant, seed: f64) -> bool {
        let Some(oldest) = self.sent.front() else { return true };
        if now.saturating_duration_since(oldest.at) > self.horizon(now) {
            if !self.late {
                self.late = true;
                self.rates.clear();
            }
            return false;
        }
        self.bytes < self.window(now, seed)
    }
}

/// Adds `value`, sampled at `now`, to `samples`, which keep a window's extreme first: the samples it `dominated` (as
/// extreme or less, so never the window's extreme again while it stands) go, then those older than `window`, and at
/// most [`MAX_RATE_SAMPLES`] stay.
fn push_extremum<T: Copy>(
    samples: &mut VecDeque<(Instant, T)>,
    now: Instant,
    value: T,
    window: Duration,
    dominated: impl Fn(T, T) -> bool,
) {
    while samples.back().is_some_and(|(_, old)| dominated(*old, value)) {
        samples.pop_back();
    }
    samples.push_back((now, value));
    while samples.len() > MAX_RATE_SAMPLES
        || samples.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) > window)
    {
        samples.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_path_change_forgets_the_flight_and_its_measurements_but_not_what_was_sent() {
        let start = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, 1000, start);
        flight.acked(1, start + MS * 200).unwrap();
        flight.sent(2, 300_000, start + MS * 210);
        flight.charge(300_000, start + MS * 210, 312_500.0);
        assert!(!flight.pacing(start + MS * 210, 312_500.0).is_zero(), "in debt for the 300 KB just sent");
        flight.path_changed();
        assert_eq!(flight.bytes(), 0);
        assert_eq!(flight.horizon(start + MS * 211), ALLOWANCE, "no round trip measured on the new path yet");
        assert!(flight.admits(start + MS * 211, 312_500.0));
        assert!(flight.pacing(start + MS * 211, 312_500.0).is_zero(), "no pacing debt carried over");
        // A late acknowledgement of the old path changes nothing; one of a revision past the newest sent is still refused.
        assert_eq!(flight.acked(2, start + MS * 400), Ok(()));
        assert_eq!(flight.acked(3, start + MS * 400), Err(Unsent));
    }

    #[test]
    fn acknowledgements_retire_what_they_cover_and_measure_the_round_trip() {
        let start = Instant::now();
        let mut flight = Flight::default();
        for (i, revision) in [3, 4, 7].into_iter().enumerate() {
            flight.sent(revision, 1000, start + MS * i as u32);
        }
        assert_eq!(flight.bytes(), 3000);
        assert_eq!(flight.acked(5, start + MS * 200), Ok(()));
        assert_eq!(flight.bytes(), 1000, "3 and 4 arrived");
        assert_eq!(flight.horizon(start + MS * 200), MS * 199 + ALLOWANCE, "from 4, sent 1 ms after the start");
        // A repeated or older acknowledgement changes nothing; one of a revision past the newest sent is refused.
        assert_eq!(flight.acked(4, start + MS * 300), Ok(()));
        assert_eq!(flight.bytes(), 1000);
        assert_eq!(flight.acked(8, start + MS * 300), Err(Unsent));
        assert_eq!(flight.acked(7, start + MS * 202), Ok(()));
        assert_eq!(flight.bytes(), 0);
        // The shortest round trip stands for its window, then the next measured replaces it.
        flight.sent(9, 10, start + MS * 300);
        flight.acked(9, start + MS * 600).unwrap();
        assert_eq!(flight.horizon(start + MS * 600), MS * 199 + ALLOWANCE);
        flight.sent(10, 10, start + RTT_WINDOW + MS * 1000);
        flight.acked(10, start + RTT_WINDOW + MS * 1250).unwrap();
        assert_eq!(flight.horizon(start + RTT_WINDOW + MS * 1250), MS * 250 + ALLOWANCE);
    }

    #[test]
    fn when_the_shortest_round_trip_leaves_the_window_the_next_shortest_stands() {
        let start = Instant::now();
        let mut flight = Flight::default();
        // 100 ms at 0.1 s, 150 ms at 9 s, then a queueing spike of 800 ms at 10.3 s.
        for (revision, (acked_ms, rtt_ms)) in [(100u64, 100u64), (9_000, 150), (10_300, 800)].into_iter().enumerate() {
            let acked = start + MS * acked_ms as u32;
            flight.sent(revision as u64 + 1, 10, acked - MS * rtt_ms as u32);
            flight.acked(revision as u64 + 1, acked).unwrap();
        }
        assert_eq!(flight.horizon(start + MS * 10_300), MS * 150 + ALLOWANCE, "the 100 ms sample expired");
        // With no acknowledgement since, the window moves on as time does: past 19 s only the latest stands.
        assert_eq!(flight.horizon(start + MS * 19_100), MS * 800 + ALLOWANCE);
        assert_eq!(flight.horizon(start + MS * 30_000), MS * 800 + ALLOWANCE, "the latest stands once all expired");
    }

    #[test]
    fn media_older_than_the_horizon_stops_capture_until_it_is_acknowledged() {
        let start = Instant::now();
        let mut flight = Flight::default();
        let seed = 312_500.0;
        assert!(flight.admits(start, seed), "nothing in flight");
        flight.sent(1, 1000, start);
        flight.acked(1, start + MS * 100).unwrap();
        flight.sent(2, 1000, start + MS * 100);
        assert!(flight.admits(start + MS * 350, seed), "250 ms old, within 100 ms + the allowance");
        assert!(!flight.admits(start + MS * 351, seed));
        // However small it is: a client that stopped acknowledging gets nothing more.
        assert!(!flight.admits(start + MS * 10_000, seed));
        flight.acked(2, start + MS * 10_000).unwrap();
        assert!(flight.admits(start + MS * 10_000, seed));
    }

    #[test]
    fn the_window_follows_the_delivery_rate_between_its_bounds() {
        let start = Instant::now();
        let mut flight = Flight::default();
        // Before any measure, the bitrate setting's rate: 2.5 Mbit/s over the allowance alone is under the least.
        assert_eq!(flight.window(start, 312_500.0), MIN_WINDOW);
        // A path that delivers 100 KB every 100 ms (8 Mbit/s) at a 200 ms round trip: 1 MB/s over 350 ms.
        let mut at = start;
        for revision in 1..=20 {
            flight.sent(revision, 100_000, at);
            if revision > 2 {
                flight.acked(revision - 2, at).unwrap();
            }
            at += MS * 100;
        }
        let window = flight.window(at, 312_500.0);
        assert!((340_000..=360_000).contains(&window), "{window}");
        // A fast path's window stops at the bound.
        let mut fast = Flight::default();
        fast.sent(1, 400_000, start);
        fast.acked(1, start + MS * 2).unwrap();
        assert_eq!(fast.window(start + MS * 2, 312_500.0), MAX_WINDOW);
        assert!(fast.admits(start + MS * 2, 312_500.0));
    }

    #[test]
    fn a_window_full_of_young_media_waits_for_acknowledgements() {
        let start = Instant::now();
        let mut flight = Flight::default();
        for revision in 1..=4 {
            flight.sent(revision, 80_000, start);
        }
        assert!(!flight.admits(start + MS * 10, 312_500.0), "320 KB in flight, over the least window");
        flight.acked(2, start + MS * 20).unwrap();
        assert!(flight.admits(start + MS * 20, 312_500.0));
    }

    #[test]
    fn media_that_outlived_the_horizon_forgets_the_rates_measured_before() {
        let start = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, 400_000, start);
        flight.acked(1, start + MS * 2).unwrap();
        assert_eq!(flight.window(start + MS * 2, 0.0), MAX_WINDOW);
        flight.sent(2, 100_000, start + MS * 10);
        assert!(!flight.admits(start + MS * 500, 0.0), "the path slowed down");
        // The next acknowledgement measures it again: 100 KB in a second.
        flight.acked(2, start + MS * 1010).unwrap();
        let rate = flight.delivery_rate(start + MS * 1010, 0.0);
        assert!((90_000.0..=110_000.0).contains(&rate), "{rate}");
    }

    #[test]
    fn what_is_sent_is_paced_at_twice_the_delivery_rate_or_the_floor() {
        let start = Instant::now();
        let mut flight = Flight::default();
        // 4 MB/s measured: the bucket refills at 8 MB/s, up to its depth.
        flight.sent(1, 400_000, start);
        flight.acked(1, start + MS * 100).unwrap();
        let now = start + MS * 100;
        assert_eq!(flight.pacing(now, 0.0), Duration::ZERO, "full after the pause");
        assert_eq!(flight.credit(now, 0.0), PACE_DEPTH as usize);
        // A burst of 800,000 bytes beyond what the bucket held leaves that much debt: 100 ms before the next.
        flight.charge(800_000 + PACE_DEPTH as usize, now, 0.0);
        let wait = flight.pacing(now, 0.0);
        assert!((MS * 99..=MS * 101).contains(&wait), "{wait:?}");
        assert_eq!(flight.credit(now, 0.0), 0);
        assert_eq!(flight.pacing(now + wait + MS, 0.0), Duration::ZERO);
        // A video's own rate (here the 2.5 Mbit/s setting, before any measure) paces at the floor: 62,500 bytes of
        // debt wait 20 ms.
        let mut fresh = Flight::default();
        fresh.charge(PACE_DEPTH as usize + 62_500, start, 312_500.0);
        assert_eq!(fresh.pacing(start, 312_500.0), MS * 20);
    }

    #[test]
    fn a_discarded_message_leaves_the_flight_alone() {
        let start = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, 1000, start);
        flight.sent(2, 2000, start);
        flight.discarded(2);
        assert_eq!(flight.bytes(), 1000);
        assert!(!flight.admits(start + Duration::from_secs(1), 0.0), "1 is still unacknowledged");
        flight.discarded(1);
        assert!(flight.admits(start + Duration::from_secs(1), 0.0), "nothing left in flight");
        assert_eq!(flight.acked(2, start + Duration::from_secs(1)), Ok(()), "an acknowledgement of a lost one");
    }
}
