use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::realtime::connect::{
    RealtimeConnectSettings, RealtimeConnectionPlan, prepare_connection,
};
use betterlite_azure::realtime::error::RealtimeError;
use betterlite_azure::realtime::http_config::{
    RealtimeHttpSettings, realtime_calls_headers, validate_environment,
};
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;

// The three HTTP session routes B545 builds beside the websocket connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRoute {
    ClientSecrets,
    Calls,
    Transcription,
}

pub struct Dispatch<'call> {
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Connection(&'call RealtimeConnectSettings<'call>),
    SessionUrl {
        route: SessionRoute,
        settings: &'call RealtimeHttpSettings,
        base: Option<&'call str>,
        version: Option<&'call str>,
    },
    SessionHeaders {
        headers: &'call Headers,
        api_key: Option<&'call SecretString>,
    },
    CallsHeaders(&'call str),
}

pub enum Outcome {
    Connection(Box<RealtimeConnectionPlan>),
    Url(String),
    Headers(Headers),
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, RealtimeError>> + Send + 'call>>;

impl Dispatch<'_> {
    // B545's websocket plan and HTTP session calls, each one read from the leaf's own settings
    pub fn run(self) -> Result<Outcome, RealtimeError> {
        Ok(match self.call {
            Call::Connection(settings) => {
                Outcome::Connection(Box::new(prepare_connection(settings)?))
            }
            Call::SessionUrl {
                route,
                settings,
                base,
                version,
            } => Outcome::Url(match route {
                SessionRoute::ClientSecrets => settings.complete_url(base, version),
                SessionRoute::Calls => settings.calls_url(base, version),
                SessionRoute::Transcription => settings.transcription_session_url(base, version),
            }),
            Call::SessionHeaders { headers, api_key } => {
                Outcome::Headers(validate_environment(headers, api_key))
            }
            Call::CallsHeaders(ephemeral_key) => {
                Outcome::Headers(realtime_calls_headers(ephemeral_key))
            }
        })
    }
}

pub fn dispatch(dispatch: Dispatch<'_>) -> RunFuture<'_> {
    Box::pin(ready(dispatch.run()))
}
