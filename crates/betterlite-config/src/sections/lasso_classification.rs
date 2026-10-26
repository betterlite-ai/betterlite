use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use super::Section;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Raw {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mask: bool,
}

impl Default for Raw {
    fn default() -> Self {
        Self { api_key: None, api_base: None, user_id: None, conversation_id: None, mask: false }
    }
}

#[derive(Debug, Clone)]
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
        let api_key = self.api_key.or_else(|| lookup.lookup("LASSO_API_KEY")).ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "LASSO_API_KEY".to_owned(),
            path: "lasso_classification.api_key".to_owned(),
        })?;
        let api_key = resolve_value(api_key, "lasso_classification.api_key", lookup)?;
        let api_base = self.api_base.or_else(|| lookup.lookup("LASSO_API_BASE"));
        let api_base = api_base.unwrap_or_else(|| "https://server.lasso.security/gateway/v3".to_owned());
        let api_base = resolve_value(api_base, "lasso_classification.api_base", lookup)?;
        let user_id = self.user_id.or_else(|| lookup.lookup("LASSO_USER_ID"));
        let conversation_id = self.conversation_id.or_else(|| lookup.lookup("LASSO_CONVERSATION_ID"));
        Ok(Settings {
            api_key: SecretString::from(api_key),
            api_base,
            user_id: user_id.map(|value| resolve_value(value, "lasso_classification.user_id", lookup)).transpose()?,
            conversation_id: conversation_id.map(|value| resolve_value(value, "lasso_classification.conversation_id", lookup)).transpose()?,
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
    lookup.lookup(variable).ok_or_else(|| ConfigError::MissingEnvironment {
        variable: variable.to_owned(),
        path: path.to_owned(),
    })
}
