use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::config::{AgentLitellmParams, parse_databricks_oauth_config};
use super::error::DatabricksOauthError;
use super::token_cache::DatabricksAppOAuthTokenCache;

// The value for the outbound Authorization header, or None when the agent has no databricks_oauth block.
pub async fn resolve_oauth_header(
    cache: &DatabricksAppOAuthTokenCache,
    params: &AgentLitellmParams,
    environment: &(dyn EnvironmentLookup + Sync),
) -> Result<Option<SecretString>, DatabricksOauthError> {
    let Some(config) = parse_databricks_oauth_config(params, environment)? else {
        return Ok(None);
    };
    let token = cache.async_get_token(&config).await?;
    Ok(Some(SecretString::from(format!(
        "Bearer {}",
        token.expose_secret()
    ))))
}
