use betterlite_models::identity::provider_id::ProviderId;
use reqwest::Client;

use super::{ClientError, ClientKey, HttpClients, Params, cached, client_builder, ssl_verify};

// get_async_httpx_client keys its cache by the provider as well as the TLS setting
pub(crate) fn async_client_for(
    clients: &mut HttpClients,
    provider: ProviderId,
    litellm_params: &Params,
    supplied: Option<Client>,
) -> Result<Client, ClientError> {
    let key = ClientKey {
        provider,
        ssl_verify: ssl_verify(litellm_params)?,
    };
    let tls = &clients.tls;
    let build = |key: ClientKey| {
        client_builder(tls, key.ssl_verify)?
            .build()
            .map_err(ClientError::Build)
    };
    cached(&mut clients.async_clients, supplied, key, build)
}
