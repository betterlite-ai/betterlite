use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use sha2::{Digest as _, Sha256};

use super::error::DatabricksOauthError;

const DEFAULT_SCOPE: &str = "all-apis";
const ENVIRONMENT_PREFIX: &str = "os.environ/";
const SERVING_ENDPOINTS_SUFFIX: &str = "/serving-endpoints";
const SECRET_DIGEST_LENGTH: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum OauthBlock {
    Mapping(BTreeMap<String, SettingValue>),
    Other(IgnoredAny),
}

#[derive(Debug, Default, Deserialize)]
pub struct AgentLitellmParams {
    #[serde(default)]
    pub databricks_oauth: Option<OauthBlock>,
}

#[derive(Clone, Debug)]
pub struct DatabricksAppOAuthConfig {
    pub client_id: String,
    pub client_secret: SecretString,
    pub token_url: String,
    pub scope: String,
}

impl DatabricksAppOAuthConfig {
    // The secret digest makes a rotated client_secret a new key, so a stale token is never served.
    #[must_use]
    pub fn cache_key(&self) -> String {
        let digest = hex::encode(Sha256::digest(
            self.client_secret.expose_secret().as_bytes(),
        ));
        let secret_digest = digest.get(..SECRET_DIGEST_LENGTH).unwrap_or(&digest);
        format!(
            "{}|{}|{}|{secret_digest}",
            self.token_url, self.client_id, self.scope
        )
    }
}

#[must_use]
pub fn resolve_secret(
    value: Option<&SettingValue>,
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    let Some(SettingValue::Text(text)) = value else {
        return None;
    };
    text.strip_prefix(ENVIRONMENT_PREFIX)
        .map_or_else(|| Some(text.clone()), |name| environment.lookup(name))
}

#[must_use]
pub fn token_url_from_workspace(workspace_url: &str) -> String {
    let stripped = workspace_url.trim().trim_end_matches('/');
    let base = stripped
        .strip_suffix(SERVING_ENDPOINTS_SUFFIX)
        .unwrap_or(stripped);
    format!("{base}/oidc/v1/token")
}

pub fn parse_databricks_oauth_config(
    params: &AgentLitellmParams,
    environment: &dyn EnvironmentLookup,
) -> Result<Option<DatabricksAppOAuthConfig>, DatabricksOauthError> {
    let settings = match &params.databricks_oauth {
        None => return Ok(None),
        Some(OauthBlock::Other(_)) => return Err(DatabricksOauthError::BlockNotMapping),
        Some(OauthBlock::Mapping(settings)) => settings,
    };
    let resolve = |name: &str| {
        resolve_secret(settings.get(name), environment).filter(|text| !text.is_empty())
    };
    match (
        resolve("client_id"),
        resolve("client_secret"),
        resolve("workspace_url"),
    ) {
        (Some(client_id), Some(client_secret), Some(workspace_url)) => {
            Ok(Some(DatabricksAppOAuthConfig {
                client_id,
                client_secret: SecretString::from(client_secret),
                token_url: token_url_from_workspace(&workspace_url),
                scope: resolve("scope").unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
            }))
        }
        (client_id, client_secret, workspace_url) => {
            let absent = [
                ("client_id", client_id.is_none()),
                ("client_secret", client_secret.is_none()),
                ("workspace_url", workspace_url.is_none()),
            ];
            Err(DatabricksOauthError::MissingFields(
                absent
                    .into_iter()
                    .filter_map(|(name, missing)| missing.then_some(name))
                    .collect(),
            ))
        }
    }
}
