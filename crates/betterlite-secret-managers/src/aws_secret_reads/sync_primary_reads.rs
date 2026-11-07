use secrecy::SecretString;

use super::{AwsBlockingTransport, AwsReadError, parse_primary_secret};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::CredentialResolver;

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver,
    Transport: AwsBlockingTransport,
{
    pub fn sync_read_from_primary(
        &self,
        secret_name: &str,
        primary_secret_name: &str,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>>
    where
        Resolver: Sync,
    {
        let primary_text = self.sync_read_secret(primary_secret_name, None, None, None)?;
        let mut secrets =
            parse_primary_secret(primary_text.as_ref()).map_err(AwsReadError::Primary)?;
        Ok(secrets.remove(secret_name))
    }
}
