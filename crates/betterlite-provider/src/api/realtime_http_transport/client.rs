use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_http::proxy::ProxiedClient;

// The pooled client a deployment would get: get_async_httpx_client reaches the OpenAI handler, whatever provider serves the model
pub trait RealtimeClientSource: Send + Sync {
    fn client(&self) -> Result<ProxiedClient, ClientError>;
}

// The client the caller handed the handler and the kind it was built as
#[derive(Debug, Clone, Copy)]
pub struct SuppliedClient<'client> {
    pub kind: ClientKind,
    pub transport: &'client ProxiedClient,
}

// An async handler the caller supplied is reused; a sync one is replaced by the pooled OpenAI client, as the isinstance check does
pub fn select(
    source: &dyn RealtimeClientSource,
    supplied: Option<SuppliedClient<'_>>,
) -> Result<ProxiedClient, ClientError> {
    if let Some(held) = supplied
        && held.kind == ClientKind::Asynchronous
    {
        return Ok(held.transport.clone());
    }
    source.client()
}
