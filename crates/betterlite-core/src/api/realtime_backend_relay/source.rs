use async_trait::async_trait;
use thiserror::Error;

use crate::api::realtime_contracts::close::ConnectionClosed;
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::realtime_session_state::logging::SuccessWork;

// LiteLLM tells a closed backend apart from a transport or decode failure, because only the close decides refusal and logging
#[derive(Debug, Error)]
pub enum RelayReceiveError {
    #[error("the realtime backend websocket closed")]
    Closed(#[source] ConnectionClosed),
    #[error("the realtime backend websocket does not accept the decode argument")]
    UnsupportedDecode,
    #[error("the realtime backend websocket receive failed")]
    Failed(#[source] WebSocketError),
}

// B2457's backend websocket as the relay reads it: the socket may be split for sending, and recv(decode=False) may be unsupported
#[async_trait]
pub trait RealtimeBackendSource: Send {
    async fn recv(&mut self, decode: Option<bool>) -> Result<RealtimeMessage, RelayReceiveError>;
}

// What B251's failure handlers receive when the backend closes before any frame: the close code and LiteLLM's exception text
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRefusal {
    pub code: u16,
    pub message: String,
}

// B251/B191's failure dispatch behind the bounded logging worker; true is LiteLLM's prefer_async_handlers for this call site
pub trait RealtimeFailureDispatch: Send + Sync {
    fn dispatch_failure_handlers(
        &self,
        refusal: &BackendRefusal,
        prefer_async_handlers: bool,
    ) -> SuccessWork;
}
