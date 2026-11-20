use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use betterlite_bedrock::realtime_transport::error::RealtimeTransportError;
use betterlite_bedrock::realtime_transport::prepare::RealtimeCredentials;
use betterlite_bedrock::realtime_transport::runtime::RealtimeRuntime;
use betterlite_bedrock::realtime_transport::session::{RealtimeCall, RealtimeTransport};

// The complete session transport already answered: the pumps ran and the client was closed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome;

// The transport error stays typed, so a caller matches its variant instead of downcasting
#[derive(Debug, thiserror::Error)]
pub enum Error<Transport>
where
    Transport: StdError + 'static,
{
    #[error("the Bedrock realtime session failed")]
    Session {
        #[source]
        source: Transport,
    },
}

pub type RunFuture<'call, Transport> =
    Pin<Box<dyn Future<Output = Result<Outcome, Error<Transport>>> + Send + 'call>>;

// B666 owns prepare, credentials, connect, the pumps and the close; this only names its session opener
#[must_use]
pub fn session<'call, Runtime, Resolver, ClientError>(
    runtime: &'call Runtime,
    credentials: &'call Resolver,
    call: RealtimeCall<'call, ClientError>,
) -> RunFuture<'call, RealtimeTransportError<Runtime::Error, ClientError>>
where
    Runtime: RealtimeRuntime,
    Resolver: RealtimeCredentials,
    ClientError: StdError + Send + Sync + 'static,
{
    Box::pin(async move {
        let outcome = RealtimeTransport::new(runtime, credentials)
            .realtime(call)
            .await;
        match outcome {
            Ok(()) => Ok(Outcome),
            Err(source) => Err(Error::Session { source }),
        }
    })
}
