use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsTransport, CredentialResolver,
};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::{DeleteOptions, SecretTimeout};

use super::SecretActionResponse;
use super::bodies::DeleteSecretBody;

const DELETE_SECRET: &str = "DeleteSecret";

pub(super) async fn delete_options<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    options: DeleteOptions<'_, AwsOptionalParams>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let DeleteOptions {
        recovery_window_in_days,
        operation,
    } = options;
    let mut optional_params = operation.optional_params.map(super::copy_params);
    delete(
        manager,
        secret_name,
        recovery_window_in_days,
        optional_params.as_mut(),
        operation.timeout,
    )
    .await
}

pub(super) async fn delete<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    recovery_window_in_days: u32,
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let body = DeleteSecretBody {
        secret_id: secret_name,
        recovery_window_in_days,
    };
    let request = ActionRequest {
        action: DELETE_SECRET,
        secret_name,
        request_data: Some(&body),
    };
    super::post_reply(manager, &request, optional_params, timeout).await
}
