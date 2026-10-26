use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Raw {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mask: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_key: SecretString,
    pub api_base: String,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mask: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let api_key = self
            .api_key
            .or_else(|| lookup.lookup("LASSO_API_KEY"))
            .ok_or_else(|| ConfigError::MissingEnvironment {
                variable: "LASSO_API_KEY".to_owned(),
                path: "lasso_classification.api_key".to_owned(),
            })?;
        let resolved_api_key = resolve_value(api_key, "lasso_classification.api_key", lookup)?;
        let configured_api_base = self.api_base.or_else(|| lookup.lookup("LASSO_API_BASE"));
        let defaulted_api_base = configured_api_base
            .unwrap_or_else(|| "https://server.lasso.security/gateway/v3".to_owned());
        let resolved_api_base =
            resolve_value(defaulted_api_base, "lasso_classification.api_base", lookup)?;
        let user_id = self.user_id.or_else(|| lookup.lookup("LASSO_USER_ID"));
        let conversation_id = self
            .conversation_id
            .or_else(|| lookup.lookup("LASSO_CONVERSATION_ID"));
        Ok(Settings {
            api_key: SecretString::from(resolved_api_key),
            api_base: resolved_api_base,
            user_id: user_id
                .map(|value| resolve_value(value, "lasso_classification.user_id", lookup))
                .transpose()?,
            conversation_id: conversation_id
                .map(|value| resolve_value(value, "lasso_classification.conversation_id", lookup))
                .transpose()?,
            mask: self.mask,
        })
    }
}

fn resolve_value<L>(value: String, path: &str, lookup: &L) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(variable) = value.strip_prefix("os.environ/") else {
        return Ok(value);
    };
    lookup
        .lookup(variable)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: variable.to_owned(),
            path: path.to_owned(),
        })
}
