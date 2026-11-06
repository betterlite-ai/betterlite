use std::time::Duration;

use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, AwsTransport, CredentialResolver,
    TransportError, TransportRequest,
};
use crate::secret_contracts::options::SecretTimeout;

const MAX_RESPONSE_BYTES: usize = 0x0010_0000;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

fn request_timeout(timeout: Option<SecretTimeout>) -> Duration {
    match timeout {
        Some(SecretTimeout::Total(limit)) => limit,
        Some(SecretTimeout::Phases {
            connect,
            read,
            write,
            pool,
        }) => [connect, read, write, pool]
            .into_iter()
            .try_fold(Duration::ZERO, |total, phase| {
                phase.map(|limit| total.saturating_add(limit))
            })
            .unwrap_or(DEFAULT_TIMEOUT),
        None => DEFAULT_TIMEOUT,
    }
}

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    pub async fn async_post_action<Body, Reply>(
        &self,
        request: &ActionRequest<'_, Body>,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
    ) -> Result<Reply, AwsRequestError<Resolver::Error, Transport::Error>>
    where
        Body: Serialize + Sync,
        Reply: DeserializeOwned,
    {
        let now = (self.dependencies.clock)();
        let prepared = self.prepare_request(request, None, optional_params, now)?;
        let response = self
            .dependencies
            .transport
            .post(TransportRequest {
                url: &prepared.endpoint_url,
                headers: prepared.headers,
                body: prepared.body.expose_secret(),
                timeout: request_timeout(timeout),
                max_response_bytes: MAX_RESPONSE_BYTES,
            })
            .await
            .map_err(|error| match error {
                TransportError::Timeout => AwsRequestError::Timeout,
                TransportError::ResponseTooLarge => AwsRequestError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                },
                TransportError::Failed(source) => AwsRequestError::Transport(source),
            })?;
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(AwsRequestError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        if !response.status.is_success() {
            let text = String::from_utf8(response.body).map_err(AwsRequestError::ErrorText)?;
            return Err(AwsRequestError::Http { text });
        }
        serde_json::from_slice(&response.body).map_err(AwsRequestError::Response)
    }
}
