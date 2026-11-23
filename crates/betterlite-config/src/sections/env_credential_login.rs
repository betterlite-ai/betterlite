mod tests;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const ENV_LOGIN_PATH: &str = "general_settings.disable_env_credential_login";
const SSO_BLOCKS_PATH: &str = "general_settings.disable_password_login_when_sso_enabled";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    disable_env_credential_login: Option<RawSetting>,
    #[serde(rename = "disable_password_login_when_sso_enabled")]
    sso_blocks_password_login: Option<RawSetting>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSetting {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}

// both settings close a login path, so only a real true turns them on and the rest of the proxy reads resolved flags
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub env_login_disabled: bool,
    pub sso_blocks_password_login: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            env_login_disabled: flag(
                self.general_settings.disable_env_credential_login,
                ENV_LOGIN_PATH,
                lookup,
            )?,
            sso_blocks_password_login: flag(
                self.general_settings.sso_blocks_password_login,
                SSO_BLOCKS_PATH,
                lookup,
            )?,
        })
    }
}

// LiteLLM reads each setting with `is True`, so no other YAML value turns it on; an os.environ/ reference is read as the flag it names.
fn flag<L>(value: Option<RawSetting>, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(setting) = value else {
        return Ok(false);
    };
    let text = match setting {
        RawSetting::Boolean(enabled) => return Ok(enabled),
        RawSetting::Text(text) => text,
        RawSetting::Other(_) => return Ok(false),
    };
    let Some(name) = text.strip_prefix("os.environ/") else {
        return Ok(false);
    };
    let resolved = resolve_string(text.to_owned(), path, lookup)?;
    Ok(betterlite_env::flag(name, &resolved)?.unwrap_or(false))
}
