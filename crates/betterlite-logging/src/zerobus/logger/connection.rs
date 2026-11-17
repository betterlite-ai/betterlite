use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use url::Url;

use crate::zerobus::client::init_params::ZerobusInitParams;
use crate::zerobus::client::types::ZerobusConnection;

pub const ENV_REFERENCE_PREFIX: &str = "os.environ/";
pub const SERVER_ENDPOINT: &str = "ZEROBUS_SERVER_ENDPOINT";
pub const WORKSPACE_URL: &str = "ZEROBUS_WORKSPACE_URL";
pub const CLIENT_ID: &str = "ZEROBUS_CLIENT_ID";
pub const CLIENT_SECRET: &str = "ZEROBUS_CLIENT_SECRET";
pub const TABLE_NAME: &str = "ZEROBUS_TABLE_NAME";

const VARIABLE_PREFIX: &str = "ZEROBUS_";

// B189 owns the general get_secret_str dispatcher; this seam is what the logger consumes until it lands
pub trait SecretSource: Send + Sync {
    fn get_secret_str(&self, name: &str) -> Option<SecretString>;
}

pub struct EnvironmentSecrets {
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl SecretSource for EnvironmentSecrets {
    fn get_secret_str(&self, name: &str) -> Option<SecretString> {
        let variable = name.strip_prefix(ENV_REFERENCE_PREFIX).unwrap_or(name);
        self.environment
            .lookup(variable)
            .filter(|text| !text.is_empty())
            .map(SecretString::from)
    }
}

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error(
        "zerobus logging requires {variable}. Set it in the environment, or callback_settings.zerobus.{key} in config.yaml"
    )]
    Missing { variable: String, key: String },
    #[error(
        "ZEROBUS_SERVER_ENDPOINT {endpoint:?} does not look like https://<workspace_id>.zerobus.<region>.cloud.databricks.com"
    )]
    WorkspaceId { endpoint: String },
    #[error("ZEROBUS_TABLE_NAME {table_name:?} must be fully qualified as catalog.schema.table")]
    TableName { table_name: String },
}

pub fn connection_for(
    params: &ZerobusInitParams,
    secrets: &dyn SecretSource,
) -> Result<ZerobusConnection, ConnectionError> {
    let server_endpoint = setting(params.server_endpoint.as_deref(), SERVER_ENDPOINT, secrets)?;
    let workspace_url = setting(params.workspace_url.as_deref(), WORKSPACE_URL, secrets)?;
    let workspace_id = workspace_id(server_endpoint.expose_secret())?;
    let client_id = setting(params.client_id.as_deref(), CLIENT_ID, secrets)?;
    let client_secret = setting(
        params
            .client_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        CLIENT_SECRET,
        secrets,
    )?;
    let table_name = table_name(params.table_name.as_deref(), secrets)?;
    Ok(ZerobusConnection {
        workspace_url: workspace_url.expose_secret().to_owned(),
        workspace_id,
        server_endpoint: server_endpoint.expose_secret().to_owned(),
        client_id: client_id.expose_secret().to_owned(),
        client_secret,
        table_name,
    })
}

pub(super) fn setting(
    configured: Option<&str>,
    variable: &str,
    secrets: &dyn SecretSource,
) -> Result<SecretString, ConnectionError> {
    let from_config = configured
        .and_then(|value| resolved_secret(value, secrets))
        .filter(|value| !value.expose_secret().is_empty());
    if let Some(value) = from_config {
        return Ok(value);
    }
    let from_environment = secrets
        .get_secret_str(variable)
        .filter(|value| !value.expose_secret().is_empty());
    if let Some(value) = from_environment {
        return Ok(value);
    }
    Err(ConnectionError::Missing {
        variable: variable.to_owned(),
        key: key_for(variable),
    })
}

pub(super) fn resolved_secret(value: &str, secrets: &dyn SecretSource) -> Option<SecretString> {
    let resolved = secrets
        .get_secret_str(value)
        .filter(|resolved| !resolved.expose_secret().is_empty());
    if resolved.is_some() {
        return resolved;
    }
    if value.starts_with(ENV_REFERENCE_PREFIX) {
        return None;
    }
    Some(SecretString::from(value))
}

fn key_for(variable: &str) -> String {
    variable
        .strip_prefix(VARIABLE_PREFIX)
        .unwrap_or(variable)
        .to_ascii_lowercase()
}

pub(super) fn workspace_id(server_endpoint: &str) -> Result<String, ConnectionError> {
    let host = Url::parse(server_endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    let label = host.split('.').next().unwrap_or_default();
    if label.is_empty() || !label.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ConnectionError::WorkspaceId {
            endpoint: server_endpoint.to_owned(),
        });
    }
    Ok(label.to_owned())
}

fn table_name(
    configured: Option<&str>,
    secrets: &dyn SecretSource,
) -> Result<String, ConnectionError> {
    let table_name = setting(configured, TABLE_NAME, secrets)?
        .expose_secret()
        .to_owned();
    if table_name.matches('.').count() != 2 {
        return Err(ConnectionError::TableName { table_name });
    }
    Ok(table_name)
}
