use std::error::Error;
use std::future::Future;

use betterlite_openai::api::responses_shapes::websocket::ResponsesWebSocketMessage;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError<Source>
where
    Source: Error + 'static,
{
    #[error("the backend does not accept the decode argument")]
    UnsupportedDecode,
    #[error("the backend connection closed: {source}")]
    Closed {
        #[source]
        source: Source,
    },
    #[error(transparent)]
    Failed {
        #[from]
        source: Source,
    },
}

pub trait NativeWsBackend: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn recv_raw(
        &self,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, BackendError<Self::Error>>> + Send + '_;
    fn recv(
        &self,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, BackendError<Self::Error>>> + Send + '_;
    fn send<'call>(
        &'call self,
        message: &'call str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'call;
    fn close(&self) -> impl Future<Output = Result<(), Self::Error>> + Send + '_;
}
