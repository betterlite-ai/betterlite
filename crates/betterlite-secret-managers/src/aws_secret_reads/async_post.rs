use secrecy::SecretString;

use super::{AwsReadError, GET_SECRET_VALUE, read_reply};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsTransport, CredentialResolver, NoBody,
};
use crate::secret_contracts::options::SecretTimeout;

pub(super) async fn post_and_read<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let request: ActionRequest<'_, NoBody> = ActionRequest {
        action: GET_SECRET_VALUE,
        secret_name,
        request_data: None,
    };
    read_reply(
        secret_name,
        manager
            .async_post_action(&request, optional_params, timeout)
            .await,
    )
}
