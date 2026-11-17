//! The HTTP client every provider request uses (13 §3): rustls, no
//! cookies, Mado's user agent.

use std::time::Duration;

/// Connecting and the first byte of an answer must come within this.
const CONNECT: Duration = Duration::from_secs(15);
/// A stream that sends nothing for this long is dropped (13 §3).
pub const IDLE: Duration = Duration::from_secs(120);

pub fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("Mado/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT)
        .read_timeout(IDLE)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
}
