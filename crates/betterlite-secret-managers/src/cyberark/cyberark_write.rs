use std::time::Duration;

use secrecy::SecretString;
use tokio::time::sleep;

use super::CyberArkSecretManager;
use super::cyberark_read::secret_url;
use super::error::CyberArkError;
use super::operation::{OperationStatus, SecretOperation};
use super::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};
use super::yaml_scalar::double_quoted;
use crate::secret_name::raise_if_unsafe_secret_name;

mod tests;

const POLICY_LOAD_ATTEMPTS: u32 = 5;
const POLICY_RETRY_DELAY: Duration = Duration::from_millis(200);
const POLICY_CONTENT_TYPE: &str = "application/x-yaml";
const CONFLICT: u16 = 409;
const UNPROCESSABLE: u16 = 422;

pub(super) async fn write<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
    secret_value: &SecretString,
) -> SecretOperation
where
    T: ConjurTransport,
{
    match store(manager, secret_name, secret_value).await {
        Ok(()) => SecretOperation {
            status: OperationStatus::Success,
            message: format!("Secret {secret_name} written successfully"),
        },
        Err(error) => {
            tracing::error!("Error writing secret to CyberArk Conjur: {error}");
            SecretOperation {
                status: OperationStatus::Error,
                message: error.to_string(),
            }
        }
    }
}

async fn store<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
    secret_value: &SecretString,
) -> Result<(), CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    raise_if_unsafe_secret_name(secret_name)?;
    ensure_variable_exists(manager, secret_name).await;
    let url = secret_url(manager, secret_name);
    post(manager, &url, None, secret_value)
        .await
        .and_then(ConjurResponse::require_success)?;
    manager.values.set(secret_name, secret_value.clone());
    Ok(())
}

async fn ensure_variable_exists<T>(manager: &CyberArkSecretManager<T>, secret_name: &str)
where
    T: ConjurTransport,
{
    let policy = SecretString::from(format!("- !variable {}\n", double_quoted(secret_name)));
    let outcome = {
        let _serialized = manager.policy_load_lock.lock().await;
        load_variable_policy(manager, &policy).await
    };
    match outcome {
        Err(error) => tracing::warn!("Error ensuring variable exists: {error}"),
        Ok(response) if (200..300).contains(&response.status) => {
            tracing::debug!("Created policy entry for variable: {secret_name}");
        }
        Ok(response) if response.status == UNPROCESSABLE => {
            tracing::debug!("Variable {secret_name} policy was rejected as unprocessable");
        }
        Ok(response) => {
            tracing::warn!("Could not ensure variable exists: {}", response.status);
        }
    }
}

async fn load_variable_policy<T>(
    manager: &CyberArkSecretManager<T>,
    policy: &SecretString,
) -> Result<ConjurResponse, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let url = format!(
        "{}/policies/{}/policy/root",
        manager.settings.api_base, manager.settings.account
    );
    let mut delay = POLICY_RETRY_DELAY;
    for _ in 1..POLICY_LOAD_ATTEMPTS {
        let response = post(manager, &url, Some(POLICY_CONTENT_TYPE), policy).await?;
        if response.status != CONFLICT {
            return Ok(response);
        }
        sleep(delay).await;
        delay = delay.saturating_mul(2);
    }
    post(manager, &url, Some(POLICY_CONTENT_TYPE), policy).await
}

async fn post<T>(
    manager: &CyberArkSecretManager<T>,
    url: &str,
    content_type: Option<&'static str>,
    body: &SecretString,
) -> Result<ConjurResponse, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let header = manager.authorization().await?;
    let request = ConjurRequest {
        method: HttpMethod::Post,
        url: url.to_owned(),
        authorization: Some(&header),
        content_type,
        body: Some(body),
        verify_tls: manager.settings.verify_tls,
        client_certificate: None,
    };
    manager
        .transport
        .send(request)
        .await
        .map_err(CyberArkError::Transport)
}
