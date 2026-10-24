//! Name resolution, done once per request by the proxy itself.

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;

/// The resolution future: every address the name resolved to.
pub type Resolution<'a> = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send + 'a>>;

/// Resolves a DNS name to all of its addresses. The proxy vets every answer and dials only one of them, so no
/// second resolution can happen between the check and the connection.
pub trait Resolver: Send + Sync {
    fn resolve<'a>(&'a self, name: &'a str, port: u16) -> Resolution<'a>;
}

/// The system resolver (`getaddrinfo` through tokio).
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve<'a>(&'a self, name: &'a str, port: u16) -> Resolution<'a> {
        Box::pin(async move {
            let mut addrs: Vec<IpAddr> = Vec::new();
            for addr in tokio::net::lookup_host((name, port)).await? {
                if !addrs.contains(&addr.ip()) {
                    addrs.push(addr.ip());
                }
            }
            Ok(addrs)
        })
    }
}
