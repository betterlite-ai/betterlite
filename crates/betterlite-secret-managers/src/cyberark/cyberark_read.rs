use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::SecretString;

use super::CyberArkSecretManager;
use super::error::CyberArkError;
use super::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};

pub(super) mod blocking;
mod tests;

// Python's quote(name, safe="") leaves only letters, digits and "_.-~" unescaped.
const NAME_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub(super) fn secret_url<T>(manager: &CyberArkSecretManager<T>, secret_name: &str) -> String {
    format!(
        "{}/secrets/{}/variable/{}",
        manager.settings.api_base,
        manager.settings.account,
        utf8_percent_encode(secret_name, NAME_ENCODING)
    )
}

pub(super) async fn fetch<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
) -> Result<ConjurResponse, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let header = manager.authorization().await?;
    let request = ConjurRequest {
        method: HttpMethod::Get,
        url: secret_url(manager, secret_name),
        authorization: Some(&header),
        content_type: None,
        body: None,
        verify_tls: manager.settings.verify_tls,
        client_certificate: None,
    };
    let reply = manager.transport.send(request).await;
    reply.map_err(CyberArkError::Transport)?.require_success()
}

pub(super) fn finish_read<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
    outcome: Result<ConjurResponse, CyberArkError<T::Error>>,
) -> Option<SecretString>
where
    T: ConjurTransport,
{
    match outcome {
        Ok(response) => {
            manager.values.set(secret_name, response.body.clone());
            Some(response.body)
        }
        Err(CyberArkError::Status { status: 404 }) => {
            tracing::debug!("Secret {secret_name} not found in CyberArk Conjur");
            None
        }
        Err(error) => {
            tracing::error!("Error reading secret from CyberArk Conjur: {error}");
            None
        }
    }
}
