//! Test-only helpers that more than one module needs.
//!
//! This is `#[cfg(test)]`-gated so nothing here reaches a release binary.

use std::io::ErrorKind;
use std::net::SocketAddr;

/// A loopback address that really refuses connections.
///
/// The obvious construction — bind `127.0.0.1:0`, read the port, drop the
/// listener, connect and expect "refused" — has a real race: between the drop
/// and the connect, another process (or another test in the same run, since the
/// suite binds ephemeral ports from several places concurrently) can take that
/// port, and the connection then succeeds or hangs instead of being refused.
/// Measured on this workspace's full library suite it failed in roughly one run
/// in thirty, as `classify_transport_error` returning `"request"` rather than
/// `"connect"`.
///
/// This binds repeatedly and returns only a port a connect has actually
/// refused. Binding is the cheap half; the loop is bounded so a pathological
/// host fails loudly instead of hanging.
pub async fn refused_loopback_address() -> SocketAddr {
    for _ in 0..64 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral loopback port cannot fail in a test");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        drop(listener);
        match tokio::net::TcpStream::connect(address).await {
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => return address,
            // Something reused the port in the window: try another one.
            _ => continue,
        }
    }
    panic!("could not obtain a refused loopback address after 64 attempts");
}
