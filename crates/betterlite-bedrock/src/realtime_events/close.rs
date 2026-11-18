use std::error::Error as StdError;

use super::stream::ClosableBedrockRuntimeClient;

// A close that fails is swallowed; only a successful close is worth a line in the log
pub async fn close_bedrock_client<E: StdError + Send + Sync + 'static>(
    client: Option<&dyn ClosableBedrockRuntimeClient<Error = E>>,
) {
    let Some(client) = client else {
        return;
    };
    if client.close().await.is_ok() {
        tracing::debug!("Bedrock Realtime: closed SDK client");
    }
}
