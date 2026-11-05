use std::collections::BTreeMap;
use std::error::Error;
use std::time::{Duration, SystemTime};

use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsActionResponse, AwsOptionalParams, AwsRequestError, CredentialResolver,
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

fn send_failure<ResolverError>(error: reqwest::Error) -> AwsRequestError<ResolverError>
where
    ResolverError: Error + 'static,
{
    if error.is_timeout() {
        AwsRequestError::Timeout
    } else {
        AwsRequestError::Transport(error)
    }
}

async fn read_bounded<ResolverError>(
    mut response: reqwest::Response,
) -> Result<Vec<u8>, AwsRequestError<ResolverError>>
where
    ResolverError: Error + 'static,
{
    let mut collected = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(send_failure)? {
        if collected.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AwsRequestError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        collected.extend_from_slice(&chunk);
    }
    Ok(collected)
}

impl<Resolver> AwsSecretsManager<Resolver>
where
    Resolver: CredentialResolver + Sync,
{
    pub async fn async_post_action(
        &self,
        request: &ActionRequest<'_>,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
    ) -> Result<AwsActionResponse, AwsRequestError<Resolver::Error>> {
        let prepared = self.prepare_request(request, None, optional_params, SystemTime::now())?;
        let response = self
            .dependencies
            .http_client
            .post(&prepared.endpoint_url)
            .headers(prepared.headers)
            .body(prepared.body.expose_secret().to_vec())
            .timeout(request_timeout(timeout))
            .send()
            .await
            .map_err(send_failure)?;
        let status = response.status();
        let bytes = read_bounded(response).await?;
        if !status.is_success() {
            let text = String::from_utf8(bytes).map_err(AwsRequestError::ErrorText)?;
            return Err(AwsRequestError::Http { text });
        }
        let fields = serde_json::from_slice::<BTreeMap<String, Box<RawValue>>>(&bytes)
            .map_err(AwsRequestError::Response)?;
        Ok(AwsActionResponse { fields })
    }
}
