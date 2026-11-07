use secrecy::SecretString;

use super::{AwsReadError, get_secret_request, parse_primary_secret, read_reply};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{AwsTransport, CredentialResolver};

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Transport: AwsTransport,
{
    pub async fn async_read_from_primary(
        &self,
        secret_name: &str,
        primary_secret_name: &str,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>>
    where
        Resolver: CredentialResolver + Sync,
    {
        let request = get_secret_request(primary_secret_name);
        let reply = self.async_post_action(&request, None, None).await;
        let primary_text = read_reply(primary_secret_name, reply)?;
        let mut secrets =
            parse_primary_secret(primary_text.as_ref()).map_err(AwsReadError::Primary)?;
        Ok(secrets.remove(secret_name))
    }
}
