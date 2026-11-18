use std::error::Error as StdError;

use super::stream::ClosableBedrockRuntimeClient;

// A close that fails is swallowed; only a successful close is worth a line in the log
pub async fn close_bedrock_client<E>(client: Option<&dyn ClosableBedrockRuntimeClient<Error = E>>)
where
    E: StdError + Send + Sync + 'static,
{
    let Some(client) = client else {
        return;
    };
    if client.close().await.is_ok() {
        tracing::debug!("Bedrock Realtime: closed SDK client");
    }
}
