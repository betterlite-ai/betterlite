pub mod cleanup;
pub mod error;
pub mod exchange;
pub mod poll;
pub mod transport;
pub mod upload;

mod tests;

use std::time::Duration;

use self::transport::{RemoteTransport, Sleeper};
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::Prepared;

pub struct SonioxRemote<'call, T: RemoteTransport, S: Sleeper> {
    pub transport: &'call T,
    pub sleeper: &'call S,
    pub prepared: &'call Prepared,
    pub timeout: Duration,
    pub limits: ApiLimits,
    // multipart boundaries come from the caller so a test and the oracle script a fixed body
    pub boundaries: fn() -> String,
}
