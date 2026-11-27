use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const TOKEN_INFO_ENDPOINT_NAME: &str = "OAUTH_TOKEN_INFO_ENDPOINT";
const USER_ID_FIELD_NAME: &str = "OAUTH_USER_ID_FIELD_NAME";
const USER_ROLE_FIELD_NAME: &str = "OAUTH_USER_ROLE_FIELD_NAME";
const USER_TEAM_ID_FIELD_NAME: &str = "OAUTH_USER_TEAM_ID_FIELD_NAME";
const CLIENT_ID_NAME: &str = "OAUTH_CLIENT_ID";
const CLIENT_SECRET_NAME: &str = "OAUTH_CLIENT_SECRET";
const CONFIG_MAPPINGS_PATH: &str = "general_settings.oauth2_config_mappings";

pub const DEFAULT_USER_ID_FIELD: &str = "sub";
pub const DEFAULT_USER_ROLE_FIELD: &str = "role";
pub const DEFAULT_USER_TEAM_ID_FIELD: &str = "team_id";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

// oauth2_config_mappings is declared dict[str, str], so a scalar entry refuses the config
#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    enable_oauth2_auth: Option<bool>,
    #[serde(default)]
    enable_oauth2_proxy_auth: Option<bool>,
    #[serde(default)]
    oauth2_config_mappings: Option<BTreeMap<String, JsonValue>>,
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub token_info_endpoint: Option<String>,
    pub user_id_field_name: String,
    pub user_role_field_name: String,
    pub user_team_id_field_name: String,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub config_mappings: Option<BTreeMap<String, String>>,
    pub enable_oauth2_auth: bool,
    pub enable_oauth2_proxy_auth: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general_settings = self.general_settings;
        Ok(Settings {
            // Oauth2Handler reads these with os.getenv/os.environ.get, so an empty variable stays an empty string
            token_info_endpoint: lookup.lookup(TOKEN_INFO_ENDPOINT_NAME),
            user_id_field_name: lookup
                .lookup(USER_ID_FIELD_NAME)
                .unwrap_or_else(|| DEFAULT_USER_ID_FIELD.to_owned()),
            user_role_field_name: lookup
                .lookup(USER_ROLE_FIELD_NAME)
                .unwrap_or_else(|| DEFAULT_USER_ROLE_FIELD.to_owned()),
            user_team_id_field_name: lookup
                .lookup(USER_TEAM_ID_FIELD_NAME)
                .unwrap_or_else(|| DEFAULT_USER_TEAM_ID_FIELD.to_owned()),
            client_id: lookup.lookup(CLIENT_ID_NAME),
            client_secret: lookup.lookup(CLIENT_SECRET_NAME).map(SecretString::from),
            config_mappings: mappings(general_settings.oauth2_config_mappings, lookup)?,
            enable_oauth2_auth: general_settings.enable_oauth2_auth.unwrap_or_default(),
            enable_oauth2_proxy_auth: general_settings
                .enable_oauth2_proxy_auth
                .unwrap_or_default(),
        })
    }
}

// The keys are the UserAPIKeyAuth identity fields and the values are the header names to read
fn mappings<L>(
    configured: Option<BTreeMap<String, JsonValue>>,
    lookup: &L,
) -> Result<Option<BTreeMap<String, String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(entries) = configured else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (field, header) in entries {
        let path = format!("{CONFIG_MAPPINGS_PATH}.{field}");
        let text = match &header {
            JsonValue::String(text) => text.as_str(),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => return Err(invalid_scalar(&path, &header)),
        };
        resolved.insert(field, resolve_string(text.to_owned(), &path, lookup)?);
    }
    Ok(Some(resolved))
}

// the mapping is dict[str, str]: a scalar of another type names its path instead of becoming text
fn invalid_scalar(path: &str, value: &JsonValue) -> ConfigError {
    let flag = match value {
        JsonValue::Bool(flag) => *flag,
        JsonValue::Null
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => false,
    };
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: flag,
    }
}
