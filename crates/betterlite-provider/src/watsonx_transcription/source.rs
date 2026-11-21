use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;

use crate::api::transcription_http::config::{ClientRequest, ClientSource};

// B173's pooled client behind the source B2453 asks for the client it sends with
pub(super) struct PooledSource {
    pub(super) client: ProxiedClient,
}

impl ClientSource for PooledSource {
    type Client = ProxiedClient;

    fn client(&self, _request: ClientRequest<'_>) -> Result<Self::Client, ClientError> {
        Ok(self.client.clone())
    }
}
