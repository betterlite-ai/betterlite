use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::SecretString;

use super::CyberArkSecretManager;
use super::cyberark_auth::{authorization, authorization_blocking};
use super::error::CyberArkError;
use super::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};

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

pub(super) async fn read<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
) -> Option<SecretString>
where
    T: ConjurTransport,
{
    if let Some(cached) = manager.values.get(secret_name) {
        return Some(cached);
    }
    let outcome = fetch(manager, secret_name).await;
    finish_read(manager, secret_name, outcome)
}

pub(super) fn read_blocking<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
) -> Option<SecretString>
where
    T: ConjurTransport,
{
    if let Some(cached) = manager.values.get(secret_name) {
        return Some(cached);
    }
    let outcome = fetch_blocking(manager, secret_name);
    finish_read(manager, secret_name, outcome)
}

async fn fetch<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
) -> Result<ConjurResponse, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let header = authorization(manager).await?;
    let url = secret_url(manager, secret_name);
    manager
        .transport
        .send(read_request(manager, &url, &header))
        .await
        .map_err(CyberArkError::Transport)
        .and_then(ConjurResponse::require_success)
}

fn fetch_blocking<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
) -> Result<ConjurResponse, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let header = authorization_blocking(manager)?;
    let url = secret_url(manager, secret_name);
    manager
        .transport
        .send_blocking(read_request(manager, &url, &header))
        .map_err(CyberArkError::Transport)
        .and_then(ConjurResponse::require_success)
}

const fn read_request<'borrowed, T>(
    manager: &'borrowed CyberArkSecretManager<T>,
    url: &'borrowed str,
    header: &'borrowed SecretString,
) -> ConjurRequest<'borrowed> {
    ConjurRequest {
        method: HttpMethod::Get,
        url,
        authorization: Some(header),
        content_type: None,
        body: None,
        verify_tls: manager.settings.verify_tls,
        client_certificate: None,
    }
}

fn finish_read<T>(
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
