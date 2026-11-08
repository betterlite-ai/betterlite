use std::error::Error;
use std::time::Duration;

use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, AwsTransport, CredentialResolver,
    TransportError, TransportRequest, TransportResponse,
};
use crate::aws_secret_reads::AwsBlockingTransport;
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

fn checked_reply<ResolverError, TransportFailure, Reply>(
    response: Result<TransportResponse, TransportError<TransportFailure>>,
) -> Result<Reply, AwsRequestError<ResolverError, TransportFailure>>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
    Reply: DeserializeOwned,
{
    let checked = response.map_err(|error| match error {
        TransportError::Timeout => AwsRequestError::Timeout,
        TransportError::ResponseTooLarge => AwsRequestError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        },
        TransportError::Failed(source) => AwsRequestError::Transport(source),
    })?;
    if checked.body.len() > MAX_RESPONSE_BYTES {
        return Err(AwsRequestError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    if !checked.status.is_success() {
        let text = String::from_utf8(checked.body).map_err(AwsRequestError::ErrorText)?;
        return Err(AwsRequestError::Http { text });
    }
    serde_json::from_slice(&checked.body).map_err(AwsRequestError::Response)
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
            .await;
        checked_reply(response)
    }
}

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsBlockingTransport,
{
    pub fn blocking_post_action<Payload, Answer>(
        &self,
        request: &ActionRequest<'_, Payload>,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
    ) -> Result<
        Answer,
        AwsRequestError<
            <Resolver as CredentialResolver>::Error,
            <Transport as AwsTransport>::Error,
        >,
    >
    where
        Answer: DeserializeOwned,
        Payload: Serialize,
    {
        let prepared =
            self.prepare_request(request, None, optional_params, (self.dependencies.clock)())?;
        checked_reply(self.dependencies.transport.post_blocking(TransportRequest {
            url: &prepared.endpoint_url,
            headers: prepared.headers,
            body: prepared.body.expose_secret(),
            timeout: request_timeout(timeout),
            max_response_bytes: MAX_RESPONSE_BYTES,
        }))
    }
}
