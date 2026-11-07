use secrecy::SecretString;

use super::{AwsReadError, get_secret_request, read_reply};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{AwsOptionalParams, AwsTransport, CredentialResolver};
use crate::secret_contracts::options::SecretTimeout;

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
{
    pub async fn async_read_secret(
        &self,
        secret_name: &str,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
        primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>>
    where
        Transport: AwsTransport,
    {
        let Some(primary) = primary_secret_name.filter(|name| !name.is_empty()) else {
            let request = get_secret_request(secret_name);
            let reply = self
                .async_post_action(&request, optional_params, timeout)
                .await;
            return read_reply(secret_name, reply);
        };
        self.async_read_from_primary(secret_name, primary).await
    }
}
