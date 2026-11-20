use std::error::Error;
use std::future::Future;

use futures_util::future::{Either, select};
use tokio_util::sync::CancellationToken;

use crate::api::realtime_contracts::close::{BackendClose, ClientLoopExit};
use crate::api::realtime_contracts::config::BaseRealtimeConfig;

use super::RealtimeClientRelay;

// LiteLLM's bidirectional_forward: the client direction wins a tie, because asyncio.wait checks client_task first
pub(crate) async fn run<Config, CallbackError, ClientDirection, BackendDirection, BackendError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    client_direction: ClientDirection,
    backend_direction: BackendDirection,
    cancellation: &CancellationToken,
) -> Result<(), BackendError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
    ClientDirection: Future<Output = ClientLoopExit>,
    BackendDirection: Future<Output = Result<BackendClose, BackendError>>,
{
    let mut client_direction = Box::pin(client_direction);
    let mut backend_direction = Box::pin(backend_direction);
    let mut client_exit = None;
    let mut backend_result = None;
    match select(client_direction.as_mut(), backend_direction.as_mut()).await {
        Either::Left((exit, _backend)) => client_exit = Some(exit),
        Either::Right((result, _client)) => backend_result = Some(result),
    }
    let client_settled = client_exit.is_some();
    let mut backend_settled = backend_result.is_some();
    let mut failure = None;
    // A disconnected client is gone: LiteLLM returns before it looks at the backend's own close
    if client_exit != Some(ClientLoopExit::ClientDisconnected) {
        let outcome = match backend_result {
            Some(result) => result,
            None => {
                backend_settled = true;
                backend_direction.as_mut().await
            }
        };
        match outcome {
            Ok(close) => relay.close_client(&close).await,
            Err(error) => failure = Some(error),
        }
    }
    cancellation.cancel();
    // A direction that ignored cancellation would hold the session open, so both are joined here rather than detached
    if !client_settled {
        let _joined_client = client_direction.as_mut().await;
    }
    if !backend_settled {
        let _joined_backend = backend_direction.as_mut().await;
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
