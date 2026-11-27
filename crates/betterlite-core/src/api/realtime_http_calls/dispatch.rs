use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::call::SuppliedClient;
use super::provider::RealtimeHttpConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEndpoint {
    ClientSecrets,
    TranscriptionSessions,
}

// The two session handlers' shared input: which endpoint, the credentials the resolver chose and the JSON body.
pub struct SessionRequest<'call> {
    pub endpoint: SessionEndpoint,
    pub provider: Option<&'call RealtimeHttpConfig>,
    pub api_base: &'call str,
    pub api_key: &'call str,
    pub request_data: &'call IndexMap<String, Json>,
    pub model: &'call str,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub api_version: Option<&'call str>,
    pub timeout: Duration,
    pub client: Option<SuppliedClient<'call>>,
}

// async_realtime_calls_handler's input: the caller's ephemeral key, the SDP offer and the session after its model rewrite.
pub struct CallsRequest<'call> {
    pub provider: Option<&'call RealtimeHttpConfig>,
    pub api_base: &'call str,
    pub ephemeral_key: &'call str,
    pub sdp: &'call [u8],
    pub model: &'call str,
    pub session: Option<&'call IndexMap<String, Json>>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub api_version: Option<&'call str>,
    pub timeout: Duration,
    pub client: Option<SuppliedClient<'call>>,
}

pub enum RealtimeHttpRequest<'call> {
    Session(SessionRequest<'call>),
    Calls(CallsRequest<'call>),
}

// The seam a provider transport fills; B2456's session_post and calls_post are the production implementation.
pub trait RealtimeHttpDispatch: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn exchange(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<UpstreamResponse, Self::Error>> + Send;
}
