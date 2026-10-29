//! Connections to a name's several addresses, staggered as RFC 8305 does: an address that drops connections silently
//! delays the next one by [`DELAY`], not by the whole connect timeout.

use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};

/// How long an attempt runs alone before the next address is tried too (RFC 8305's connection attempt delay).
const DELAY: Duration = Duration::from_millis(250);

/// Runs `attempt` on each of `addrs` in order, each starting once the one before failed or ran for [`DELAY`]: the first
/// to succeed is returned with its address, and the others are dropped. Every failure, if none succeeds.
pub async fn first<A: Copy, T, E, F: Future<Output = Result<T, E>>>(
    addrs: &[A],
    attempt: impl Fn(A) -> F,
) -> Result<(T, A), Vec<E>> {
    let mut rest = addrs.iter().copied();
    let mut next = rest.next();
    let mut running = FuturesUnordered::new();
    let mut failures = Vec::new();
    loop {
        if let Some(addr) = next.take() {
            let attempt = attempt(addr);
            running.push(async move { (addr, attempt.await) });
        }
        let more = rest.len() > 0;
        tokio::select! {
            Some((addr, result)) = running.next() => match result {
                Ok(value) => return Ok((value, addr)),
                Err(e) => {
                    failures.push(e);
                    next = rest.next();
                }
            },
            () = tokio::time::sleep(DELAY), if more => next = rest.next(),
            else => return Err(failures),
        }
    }
}
