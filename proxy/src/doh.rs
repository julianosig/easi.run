//! Name resolution through the upstream: DNS over HTTPS (Cloudflare's JSON API at 1.1.1.1, its certificate verified for
//! that address) over one HTTP/2 connection tunnelled through the upstream, so no name is resolved from the proxy's
//! own network and the answers fit where the traffic leaves. The proxy vets every answer as it vets the system
//! resolver's.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::client::conn::http2::SendRequest;
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::Deserialize;
use tokio::sync::{Semaphore, watch};

use crate::resolve::{Resolution, Resolver};
use crate::upstream::{Dialer, Target, tls, tls_config};

/// The resolver: Cloudflare's, by address, so finding it needs no resolution.
const SERVER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 443);
const SERVER_NAME: &str = "1.1.1.1";
/// The largest answer read, in bytes, and the most records it may hold.
const MAX_ANSWER: usize = 64 * 1024;
const MAX_RECORDS: usize = 64;
/// Queries at once (concurrent lookups of one name share one), and names remembered.
const MAX_QUERIES: usize = 16;
const MAX_CACHED: usize = 4096;
/// The longest an answer is remembered, whatever its TTL.
const MAX_TTL: u32 = 3600;
/// How long a name without an address is remembered: briefly, so a page that keeps asking for a dead host does not
/// send a query through the upstream for every request.
const NEGATIVE_TTL: u32 = 30;
/// The longest one query may take, building the shared connection included, well within the proxy's own wait for a
/// name (whose timeout would otherwise cancel the query first): a connection that stopped answering without closing is
/// replaced only once a query on it times out here, and a build that stalls is abandoned.
const QUERY_TIMEOUT: Duration = Duration::from_secs(4);
const A: u16 = 1;
const CNAME: u16 = 5;
const AAAA: u16 = 28;

/// The JSON answer (only the fields used; the rest is ignored).
#[derive(Deserialize)]
struct Reply {
    #[serde(rename = "Status")]
    status: u32,
    #[serde(rename = "TC", default)]
    truncated: bool,
    #[serde(rename = "Answer", default)]
    answer: Vec<Record>,
}

#[derive(Deserialize)]
struct Record {
    #[serde(rename = "type")]
    kind: u16,
    #[serde(rename = "TTL")]
    ttl: u32,
    data: String,
}

/// What one query learned.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// The addresses of the asked type (possibly none) and their smallest TTL.
    Records(Vec<IpAddr>, u32),
    /// NXDOMAIN: the name does not exist, for any type (RFC 8020).
    NoSuchName,
}

/// The shared connection, if any, and the generation of the last one built: a query that fails forgets the connection
/// only if it is still the one the query used, never a newer one another query built since.
#[derive(Default)]
struct Slot {
    sender: Option<SendRequest<Empty<Bytes>>>,
    generation: u64,
}

/// A lookup's outcome as every lookup of the name waiting on it receives it: `None` until it is known.
type Outcome = Option<Result<Vec<IpAddr>, (io::ErrorKind, String)>>;

/// The lookups in flight, by name: the first lookup of a name queries, and the others wait for its outcome.
type Flights = Mutex<HashMap<String, watch::Receiver<Outcome>>>;

/// The querying lookup's hold on its name: dropped, finished or cancelled, it removes the name from the flights, and
/// a lookup still waiting without an outcome then tries again.
struct Flight<'a> {
    flights: &'a Flights,
    name: &'a str,
    outcome: watch::Sender<Outcome>,
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        self.flights.lock().unwrap().remove(self.name);
    }
}

pub struct Doh {
    dialer: Arc<Dialer>,
    tls: Arc<rustls::ClientConfig>,
    connection: tokio::sync::Mutex<Slot>,
    cache: Mutex<HashMap<String, (Vec<IpAddr>, Instant)>>,
    flights: Flights,
    queries: Semaphore,
}

impl Doh {
    pub fn new(dialer: Arc<Dialer>) -> io::Result<Self> {
        Ok(Self {
            dialer,
            tls: tls_config(&[b"h2"])?,
            connection: tokio::sync::Mutex::new(Slot::default()),
            cache: Mutex::new(HashMap::new()),
            flights: Mutex::new(HashMap::new()),
            queries: Semaphore::new(MAX_QUERIES),
        })
    }

    /// The addresses of `name`: remembered, else the outcome of the lookup of it in flight, else queried.
    async fn lookup(&self, name: &str) -> io::Result<Vec<IpAddr>> {
        loop {
            // Ok: this lookup queries; Err: it waits for the one that does.
            let role = {
                let mut flights = self.flights.lock().unwrap();
                if let Some((addrs, until)) = self.cache.lock().unwrap().get(name)
                    && Instant::now() < *until
                {
                    return Ok(addrs.clone());
                }
                match flights.entry(name.to_owned()) {
                    Entry::Occupied(flight) => Err(flight.get().clone()),
                    Entry::Vacant(slot) => {
                        let (outcome, receiver) = watch::channel(None);
                        slot.insert(receiver);
                        Ok(Flight { flights: &self.flights, name, outcome })
                    }
                }
            };
            match role {
                Ok(flight) => {
                    let result = self.fetch(name).await;
                    let shared = result.as_ref().map(Vec::clone).map_err(|e| (e.kind(), e.to_string()));
                    flight.outcome.send_replace(Some(shared));
                    return result;
                }
                Err(mut receiver) => {
                    if let Ok(outcome) = receiver.wait_for(Option::is_some).await
                        && let Some(result) = outcome.as_ref()
                    {
                        return result.clone().map_err(|(kind, message)| io::Error::new(kind, message));
                    }
                }
            }
        }
    }

    /// Queries the addresses of `name` (A, else AAAA, unless the name does not exist) and remembers them for their
    /// TTL, or that there are none for [`NEGATIVE_TTL`].
    async fn fetch(&self, name: &str) -> io::Result<Vec<IpAddr>> {
        let _permit = self.queries.acquire().await.map_err(io::Error::other)?;
        let (addrs, ttl) = match self.query(name, A).await? {
            Answer::Records(addrs, ttl) if !addrs.is_empty() => (addrs, ttl),
            Answer::NoSuchName => (Vec::new(), NEGATIVE_TTL),
            Answer::Records(..) => match self.query(name, AAAA).await? {
                Answer::Records(addrs, ttl) if !addrs.is_empty() => (addrs, ttl),
                _ => (Vec::new(), NEGATIVE_TTL),
            },
        };
        if ttl > 0 {
            let mut cache = self.cache.lock().unwrap();
            let now = Instant::now();
            if cache.len() >= MAX_CACHED {
                cache.retain(|_, (_, until)| *until > now);
            }
            if cache.len() < MAX_CACHED {
                let until = now + Duration::from_secs(ttl.min(MAX_TTL).into());
                cache.insert(name.to_owned(), (addrs.clone(), until));
            }
        }
        Ok(addrs)
    }

    /// The addresses of `kind` for `name`, and the smallest TTL along the answer (its CNAMEs included).
    async fn query(&self, name: &str, kind: u16) -> io::Result<Answer> {
        let deadline = tokio::time::Instant::now() + QUERY_TIMEOUT;
        let (generation, mut sender) = match tokio::time::timeout_at(deadline, self.sender()).await {
            Ok(connection) => connection?,
            Err(_) => return Err(io::ErrorKind::TimedOut.into()),
        };
        let kind_name = if kind == A { "A" } else { "AAAA" };
        let request = hyper::Request::get(format!("https://{SERVER_NAME}/dns-query?name={name}&type={kind_name}"))
            .header(hyper::header::ACCEPT, "application/dns-json")
            .body(Empty::new())
            .map_err(io::Error::other)?;
        let response = match tokio::time::timeout_at(deadline, sender.send_request(request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => return Err(self.drop_connection(generation, io::Error::other(error)).await),
            Err(_) => return Err(self.drop_connection(generation, io::ErrorKind::TimedOut.into()).await),
        };
        if response.status() != hyper::StatusCode::OK {
            return Err(io::Error::other("the resolver answered with an error status"));
        }
        let body = Limited::new(response.into_body(), MAX_ANSWER).collect();
        let body = match tokio::time::timeout_at(deadline, body).await {
            Ok(body) => body.map_err(io::Error::other)?,
            Err(_) => return Err(self.drop_connection(generation, io::ErrorKind::TimedOut.into()).await),
        };
        parse(&body.to_bytes(), kind)
    }

    /// Forgets the shared connection of `generation`, broken or no longer answering, so the next query builds another;
    /// returns `error`.
    async fn drop_connection(&self, generation: u64, error: io::Error) -> io::Error {
        let mut connection = self.connection.lock().await;
        if connection.generation == generation {
            connection.sender = None;
        }
        error
    }

    /// The shared connection and its generation, built (through the upstream, then TLS) when there is none or it
    /// closed.
    async fn sender(&self) -> io::Result<(u64, SendRequest<Empty<Bytes>>)> {
        let mut connection = self.connection.lock().await;
        if let Some(sender) = connection.sender.as_ref()
            && !sender.is_closed()
        {
            return Ok((connection.generation, sender.clone()));
        }
        let tunnel = self.dialer.dial(Target::Addr(SERVER)).await?;
        let stream = tls(tunnel, SERVER_NAME, self.tls.clone()).await?;
        let (sender, driver) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
            .await
            .map_err(io::Error::other)?;
        tokio::spawn(driver);
        connection.generation += 1;
        connection.sender = Some(sender.clone());
        Ok((connection.generation, sender))
    }
}

/// The addresses of `kind` in a JSON answer and its smallest TTL, or that the name does not exist; an error for
/// anything else but a complete success.
fn parse(body: &[u8], kind: u16) -> io::Result<Answer> {
    let reply: Reply = serde_json::from_slice(body).map_err(io::Error::other)?;
    // NXDOMAIN (3) is an answer: there is no such name.
    if reply.status == 3 {
        return Ok(Answer::NoSuchName);
    }
    if reply.status != 0 || reply.truncated || reply.answer.len() > MAX_RECORDS {
        return Err(io::Error::other("the resolver's answer is not a complete success"));
    }
    let mut addrs = Vec::new();
    let mut ttl = MAX_TTL;
    for record in &reply.answer {
        match record.kind {
            k if k == kind => {
                let addr: IpAddr = record.data.parse().map_err(|_| io::Error::other("not an address"))?;
                if matches!((kind, addr), (A, IpAddr::V4(_)) | (AAAA, IpAddr::V6(_))) && !addrs.contains(&addr) {
                    addrs.push(addr);
                }
            }
            CNAME => {}
            _ => continue,
        }
        ttl = ttl.min(record.ttl);
    }
    let ttl = if addrs.is_empty() { 0 } else { ttl };
    Ok(Answer::Records(addrs, ttl))
}

impl Resolver for Doh {
    fn resolve<'a>(&'a self, name: &'a str, _port: u16) -> Resolution<'a> {
        Box::pin(self.lookup(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_give_their_addresses_and_smallest_ttl() {
        let body = br#"{"Status":0,"TC":false,"Answer":[
            {"name":"www.example.com","type":5,"TTL":120,"data":"example.com."},
            {"name":"example.com","type":1,"TTL":300,"data":"93.184.216.34"},
            {"name":"example.com","type":1,"TTL":600,"data":"93.184.216.35"}]}"#;
        let addrs = vec!["93.184.216.34".parse::<IpAddr>().unwrap(), "93.184.216.35".parse().unwrap()];
        assert_eq!(parse(body, A).unwrap(), Answer::Records(addrs, 120));
        let v6 = br#"{"Status":0,"Answer":[{"name":"x","type":28,"TTL":0,"data":"2606:2800::1"}]}"#;
        assert_eq!(parse(v6, AAAA).unwrap(), Answer::Records(vec!["2606:2800::1".parse().unwrap()], 0));
        assert_eq!(parse(br#"{"Status":3}"#, A).unwrap(), Answer::NoSuchName);
    }

    #[test]
    fn failures_truncation_and_garbage_are_errors() {
        for body in [
            &br#"{"Status":2}"#[..],
            br#"{"Status":0,"TC":true,"Answer":[]}"#,
            br#"{"Status":0,"Answer":[{"type":1,"TTL":5,"data":"not-an-ip"}]}"#,
            b"<html>",
        ] {
            assert!(parse(body, A).is_err(), "{}", String::from_utf8_lossy(body));
        }
        let many: Vec<String> =
            (0..=MAX_RECORDS).map(|i| format!(r#"{{"type":1,"TTL":5,"data":"1.2.3.{}"}}"#, i % 250)).collect();
        let body = format!(r#"{{"Status":0,"Answer":[{}]}}"#, many.join(","));
        assert!(parse(body.as_bytes(), A).is_err());
    }

    fn doh(url: &str) -> Doh {
        Doh::new(Arc::new(Dialer::new(egress::Upstream::parse_url(url).unwrap()).unwrap())).unwrap()
    }

    /// A resolver whose connection is already `sender`, at generation 1.
    async fn doh_on(sender: SendRequest<Empty<Bytes>>) -> Doh {
        let doh = doh("http://127.0.0.1:9");
        *doh.connection.lock().await = Slot { sender: Some(sender), generation: 1 };
        doh
    }

    /// An HTTP/2 connection whose server takes the preface and then never answers, nor closes.
    async fn silent_connection() -> (SendRequest<Empty<Bytes>>, tokio::io::DuplexStream) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (sender, driver) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(client)).await.unwrap();
        tokio::spawn(driver);
        (sender, server)
    }

    #[tokio::test]
    async fn a_connection_that_stops_answering_is_replaced() {
        let (sender, _server) = silent_connection().await;
        let doh = doh_on(sender).await;
        let error = doh.query("example.com", A).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(doh.connection.lock().await.sender.is_none(), "the next query builds another connection");
    }

    #[tokio::test]
    async fn a_connection_build_that_stalls_is_abandoned_at_the_deadline() {
        // An upstream that takes the connection and never answers its CONNECT.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let doh = doh(&format!("http://{}", listener.local_addr().unwrap()));
        let started = tokio::time::Instant::now();
        let error = doh.query("example.com", A).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < QUERY_TIMEOUT + Duration::from_secs(1));
        let connection = doh.connection.try_lock().expect("the stalled build no longer holds the connection");
        assert!(connection.sender.is_none());
    }

    /// Starts `count` lookups of one name, on a connection that never answers, and lets them reach their wait.
    async fn lookups(doh: &Arc<Doh>, count: usize) -> Vec<tokio::task::JoinHandle<io::Result<Vec<IpAddr>>>> {
        let handles = (0..count)
            .map(|_| {
                let doh = doh.clone();
                tokio::spawn(async move { doh.lookup("example.com").await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(100)).await;
        handles
    }

    #[tokio::test]
    async fn concurrent_lookups_of_a_name_share_one_query() {
        let (sender, _server) = silent_connection().await;
        let doh = Arc::new(doh_on(sender).await);
        let handles = lookups(&doh, 8).await;
        assert_eq!(doh.queries.available_permits(), MAX_QUERIES - 1, "one query for every lookup of the name");
        assert_eq!(doh.flights.lock().unwrap().len(), 1);
        for handle in handles {
            assert_eq!(handle.await.unwrap().unwrap_err().kind(), io::ErrorKind::TimedOut, "each gets its outcome");
        }
        assert!(doh.flights.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_waiting_lookup_queries_itself_when_the_querying_one_is_cancelled() {
        let (sender, _server) = silent_connection().await;
        let doh = Arc::new(doh_on(sender).await);
        let leader = lookups(&doh, 1).await;
        let waiter = lookups(&doh, 1).await;
        leader[0].abort();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(doh.queries.available_permits(), MAX_QUERIES - 1, "the waiting lookup took over the query");
        assert_eq!(doh.flights.lock().unwrap().len(), 1);
        let [waiter] = <[_; 1]>::try_from(waiter).unwrap();
        assert_eq!(waiter.await.unwrap().unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(doh.flights.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_query_leaves_a_newer_connection_alone() {
        let (sender, _server) = silent_connection().await;
        let doh = doh_on(sender).await;
        doh.connection.lock().await.generation = 2;
        doh.drop_connection(1, io::ErrorKind::TimedOut.into()).await;
        assert!(
            doh.connection.lock().await.sender.is_some(),
            "an older query's failure forgets only its own connection"
        );
        doh.drop_connection(2, io::ErrorKind::TimedOut.into()).await;
        assert!(doh.connection.lock().await.sender.is_none());
    }

    #[test]
    fn an_address_of_the_other_family_is_ignored() {
        let body = br#"{"Status":0,"Answer":[{"type":1,"TTL":5,"data":"2606:2800::1"}]}"#;
        assert_eq!(parse(body, A).unwrap(), Answer::Records(vec![], 0));
    }

    /// An HTTP/2 connection to a resolver that answers each query with `answer(type)` and records the types asked.
    async fn answering_connection(
        answer: fn(&str) -> &'static str,
    ) -> (SendRequest<Empty<Bytes>>, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let query = request.uri().query().unwrap_or_default();
            let kind = query.rsplit_once("type=").map(|(_, kind)| kind.to_owned()).unwrap_or_default();
            let body = answer(&kind);
            log.lock().unwrap().push(kind);
            async move {
                Ok::<_, std::convert::Infallible>(hyper::Response::new(http_body_util::Full::new(Bytes::from(body))))
            }
        });
        let (client, server) = tokio::io::duplex(64 * 1024);
        tokio::spawn(
            hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(server), service),
        );
        let (sender, driver) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(client)).await.unwrap();
        tokio::spawn(driver);
        (sender, asked)
    }

    #[tokio::test]
    async fn a_name_that_does_not_exist_is_asked_once_and_remembered_briefly() {
        let (sender, asked) = answering_connection(|_| r#"{"Status":3}"#).await;
        let doh = doh_on(sender).await;
        assert!(doh.lookup("dead.example").await.unwrap().is_empty());
        assert_eq!(*asked.lock().unwrap(), ["A"], "NXDOMAIN holds for AAAA too");
        assert!(doh.lookup("dead.example").await.unwrap().is_empty());
        assert_eq!(asked.lock().unwrap().len(), 1, "the negative answer is remembered");
        let (_, until) = doh.cache.lock().unwrap()["dead.example"].clone();
        assert!(until <= Instant::now() + Duration::from_secs(NEGATIVE_TTL.into()));
    }

    #[tokio::test]
    async fn a_name_without_an_ipv4_address_is_asked_for_ipv6_and_an_empty_answer_is_remembered() {
        let (sender, asked) = answering_connection(|_| r#"{"Status":0,"Answer":[]}"#).await;
        let empty = doh_on(sender).await;
        assert!(empty.lookup("empty.example").await.unwrap().is_empty());
        assert!(empty.lookup("empty.example").await.unwrap().is_empty());
        assert_eq!(*asked.lock().unwrap(), ["A", "AAAA"]);

        let (sender, asked) = answering_connection(|kind| match kind {
            "AAAA" => r#"{"Status":0,"Answer":[{"type":28,"TTL":300,"data":"2606:2800::1"}]}"#,
            _ => r#"{"Status":0,"Answer":[]}"#,
        })
        .await;
        let doh = doh_on(sender).await;
        let v6: IpAddr = "2606:2800::1".parse().unwrap();
        assert_eq!(doh.lookup("v6.example").await.unwrap(), [v6]);
        assert_eq!(doh.lookup("v6.example").await.unwrap(), [v6]);
        assert_eq!(*asked.lock().unwrap(), ["A", "AAAA"]);
    }
}
