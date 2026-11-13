use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsTransport, CredentialResolver,
};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::SecretOptions;

use super::SecretActionResponse;
use super::bodies::PutSecretValueBody;

const PUT_SECRET_VALUE: &str = "PutSecretValue";

pub async fn async_put_secret_value<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    secret_value: &SecretString,
    options: SecretOptions<'_, AwsOptionalParams>,
    next_token: fn() -> Uuid,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let token = next_token().to_string();
    let body = PutSecretValueBody {
        secret_id: secret_name,
        secret_string: secret_value.expose_secret(),
        client_request_token: &token,
    };
    let request = ActionRequest {
        action: PUT_SECRET_VALUE,
        secret_name,
        request_data: Some(&body),
    };
    let mut optional_params = options.optional_params.map(super::copy_params);
    super::post_reply(manager, &request, optional_params.as_mut(), options.timeout).await
}
