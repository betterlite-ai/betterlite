use secrecy::SecretString;
use serde::Serialize;
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsTransport, CredentialResolver,
};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::SecretTimeout;

mod bodies;
mod delete;
mod manager;
mod restore;
mod rotate;
mod tests;
mod write;

// the trait carries write, delete and rotate; replication and in-place updates are called directly
pub mod put_value;
pub mod replicate;

// LiteLLM declares the write, restore, replicate, put and delete replies as a free-form dict, so the body stays verbatim
#[derive(Debug, Clone)]
pub struct SecretActionResponse {
    pub body: SecretString,
}

// The token source is a field so a test or the oracle can pin every ClientRequestToken a write sends
pub struct AwsSecretClient<'borrowed, Resolver, Transport> {
    pub manager: &'borrowed AwsSecretsManager<Resolver, Transport>,
    pub next_token: fn() -> Uuid,
}

pub(crate) async fn post_reply<Body, Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    request: &ActionRequest<'_, Body>,
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Body: Serialize + Sync,
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let reply: Box<RawValue> = manager
        .async_post_action(request, optional_params, timeout)
        .await
        .map_err(AwsReadError::Request)?;
    Ok(SecretActionResponse {
        body: SecretString::from(reply.get().to_owned()),
    })
}

// _prepare_request fills a caller's optional_params in place; the trait hands us a shared reference
pub(crate) fn copy_params(params: &AwsOptionalParams) -> AwsOptionalParams {
    AwsOptionalParams {
        aws_role_name: params.aws_role_name.clone(),
        aws_session_name: params.aws_session_name.clone(),
        aws_region_name: params.aws_region_name.clone(),
        aws_external_id: params.aws_external_id.clone(),
        aws_profile_name: params.aws_profile_name.clone(),
        aws_web_identity_token: params.aws_web_identity_token.clone(),
        aws_sts_endpoint: params.aws_sts_endpoint.clone(),
        aws_access_key_id: params.aws_access_key_id.clone(),
        aws_secret_access_key: params.aws_secret_access_key.clone(),
        aws_session_token: params.aws_session_token.clone(),
        aws_bedrock_runtime_endpoint: params.aws_bedrock_runtime_endpoint.clone(),
    }
}
