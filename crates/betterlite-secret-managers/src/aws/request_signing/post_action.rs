use std::time::Duration;

use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, AwsTransport, CredentialResolver,
    TransportError, TransportRequest, TransportResponse,
};
use crate::secret_contracts::options::SecretTimeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    pub(crate) const MAX_RESPONSE_BYTES: usize = 0x0010_0000;

    pub(crate) fn request_timeout(timeout: Option<SecretTimeout>) -> Duration {
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

    pub(crate) fn checked_reply<Reply>(
        response: Result<TransportResponse, TransportError<Transport::Error>>,
    ) -> Result<Reply, AwsRequestError<Resolver::Error, Transport::Error>>
    where
        Reply: DeserializeOwned,
    {
        let checked = response.map_err(|error| match error {
            TransportError::Timeout => AwsRequestError::Timeout,
            TransportError::ResponseTooLarge => AwsRequestError::ResponseTooLarge {
                limit: Self::MAX_RESPONSE_BYTES,
            },
            TransportError::Failed(source) => AwsRequestError::Transport(source),
        })?;
        if checked.body.len() > Self::MAX_RESPONSE_BYTES {
            return Err(AwsRequestError::ResponseTooLarge {
                limit: Self::MAX_RESPONSE_BYTES,
            });
        }
        if !checked.status.is_success() {
            let text = String::from_utf8(checked.body).map_err(AwsRequestError::ErrorText)?;
            return Err(AwsRequestError::Http { text });
        }
        serde_json::from_slice(&checked.body).map_err(AwsRequestError::Response)
    }

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
                timeout: Self::request_timeout(timeout),
                max_response_bytes: Self::MAX_RESPONSE_BYTES,
            })
            .await;
        Self::checked_reply(response)
    }
}
