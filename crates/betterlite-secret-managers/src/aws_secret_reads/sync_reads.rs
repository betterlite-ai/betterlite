use secrecy::SecretString;

use super::{AwsBlockingTransport, AwsReadError, get_secret_request, read_reply};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{AwsOptionalParams, CredentialResolver};
use crate::secret_contracts::options::SecretTimeout;

const BOOTSTRAP_NAMES: [&str; 5] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_REGION_NAME",
    "AWS_REGION",
    "AWS_BEDROCK_RUNTIME_ENDPOINT",
];

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Transport: AwsBlockingTransport,
{
    pub fn sync_read_secret(
        &self,
        secret_name: &str,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
        primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>>
    where
        Resolver: CredentialResolver + Sync,
    {
        if BOOTSTRAP_NAMES.contains(&secret_name) {
            let value = self.dependencies.environment.get(secret_name).cloned();
            return Ok(value.map(SecretString::from));
        }
        let Some(primary) = primary_secret_name.filter(|name| !name.is_empty()) else {
            let request = get_secret_request(secret_name);
            let reply = self.blocking_post_action(&request, optional_params, timeout);
            return read_reply(secret_name, reply);
        };
        self.sync_read_from_primary(secret_name, primary)
    }
}
