use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsTransport, CredentialResolver,
};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::SecretTimeout;

use super::SecretActionResponse;
use super::bodies::PutSecretValueBody;

const PUT_SECRET_VALUE: &str = "PutSecretValue";

pub async fn async_put_secret_value<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    secret_value: &SecretString,
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let token = Uuid::new_v4().to_string();
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
    super::post_reply(manager, &request, optional_params, timeout).await
}
