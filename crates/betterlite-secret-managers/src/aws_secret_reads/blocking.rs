use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{AwsReadError, BOOTSTRAP_NAMES, GET_SECRET_VALUE, parse_primary_secret, read_reply};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, AwsTransport, CredentialResolver, NoBody,
    TransportError, TransportRequest, TransportResponse,
};
use crate::secret_contracts::options::SecretTimeout;

pub trait AwsBlockingTransport: AwsTransport {
    fn post_blocking(
        &self,
        request: TransportRequest<'_>,
    ) -> Result<TransportResponse, TransportError<Self::Error>>;
}

impl<Resolver, Transport> AwsSecretsManager<Resolver, Transport>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsBlockingTransport,
{
    pub fn blocking_post_action<Body, Reply>(
        &self,
        request: &ActionRequest<'_, Body>,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
    ) -> Result<Reply, AwsRequestError<Resolver::Error, Transport::Error>>
    where
        Body: Serialize + Sync,
        Reply: DeserializeOwned,
    {
        let now = (self.dependencies.clock)();
        let prepared = self.prepare_request(request, None, optional_params, now)?;
        let response = self.dependencies.transport.post_blocking(TransportRequest {
            url: &prepared.endpoint_url,
            headers: prepared.headers,
            body: prepared.body.expose_secret(),
            timeout: Self::request_timeout(timeout),
            max_response_bytes: Self::MAX_RESPONSE_BYTES,
        });
        Self::checked_reply(response)
    }

    pub fn sync_read_secret(
        &self,
        secret_name: &str,
        optional_params: Option<&mut AwsOptionalParams>,
        timeout: Option<SecretTimeout>,
        primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>> {
        if BOOTSTRAP_NAMES.contains(&secret_name) {
            let value = self.dependencies.environment.get(secret_name).cloned();
            return Ok(value.map(SecretString::from));
        }
        let Some(primary) = primary_secret_name.filter(|name| !name.is_empty()) else {
            let request: ActionRequest<'_, NoBody> = ActionRequest {
                action: GET_SECRET_VALUE,
                secret_name,
                request_data: None,
            };
            let reply = self.blocking_post_action(&request, optional_params, timeout);
            return read_reply(secret_name, reply);
        };
        self.sync_read_from_primary(secret_name, primary)
    }

    pub fn sync_read_from_primary(
        &self,
        secret_name: &str,
        primary_secret_name: &str,
    ) -> Result<Option<SecretString>, AwsReadError<Resolver::Error, Transport::Error>> {
        let primary_text = self.sync_read_secret(primary_secret_name, None, None, None)?;
        let mut secrets =
            parse_primary_secret(primary_text.as_ref()).map_err(AwsReadError::Primary)?;
        Ok(secrets.remove(secret_name))
    }
}
