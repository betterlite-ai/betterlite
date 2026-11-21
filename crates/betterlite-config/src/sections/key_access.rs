mod tests;

use betterlite_secret_managers::secret_lookup::text_to_bool;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use crate::sections::settings::coerce_bool::coerce_bool;

const DISABLE_CUSTOM_KEYS_PATH: &str = "general_settings.disable_custom_api_keys";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub general_settings: General,
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct General {
    #[serde(default)]
    pub disable_custom_api_keys: JsonValue,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    #[serde(default)]
    pub key_generation_settings: Option<StandardKeyGeneration>,
}

// StandardKeyGenerationConfig is a total=False TypedDict, so every entry may be left out
#[derive(Debug, serde::Deserialize)]
pub struct StandardKeyGeneration {
    #[serde(default)]
    pub team_key_generation: Option<TeamKeyGeneration>,
    #[serde(default)]
    pub personal_key_generation: Option<PersonalKeyGeneration>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct TeamKeyGeneration {
    #[serde(default)]
    pub allowed_team_member_roles: Option<Vec<String>>,
    #[serde(default)]
    pub required_params: Option<Vec<String>>,
}

// the TeamUIKeyGenerationConfig _team_key_generation_check builds when the setting names no team entry
impl Default for TeamKeyGeneration {
    fn default() -> Self {
        Self {
            allowed_team_member_roles: Some(vec!["admin".to_owned(), "user".to_owned()]),
            required_params: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PersonalKeyGeneration {
    #[serde(default)]
    pub allowed_user_roles: Option<Vec<String>>,
    #[serde(default)]
    pub required_params: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyGeneration {
    // litellm.key_generation_settings is not None, so a team the database lacks fails even for proxy admins
    pub configured: bool,
    pub team: TeamKeyGeneration,
    pub personal: Option<PersonalKeyGeneration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub disable_custom_api_keys: bool,
    pub key_generation: KeyGeneration,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let switch = match self.general_settings.disable_custom_api_keys {
            // get_secret turns trimmed true or false env text into a bool before coerce_bool sees it
            JsonValue::String(text) if text.starts_with("os.environ/") => {
                let resolved = resolve_string(text, DISABLE_CUSTOM_KEYS_PATH, lookup)?;
                JsonValue::Bool(text_to_bool(&resolved) == Some(true))
            }
            other => other,
        };
        let key_generation = self.litellm_settings.key_generation_settings.map_or_else(
            KeyGeneration::default,
            |configured| KeyGeneration {
                configured: true,
                team: configured.team_key_generation.unwrap_or_default(),
                personal: configured.personal_key_generation,
            },
        );
        Ok(Settings {
            disable_custom_api_keys: matches!(coerce_bool(switch), JsonValue::Bool(true)),
            key_generation,
        })
    }
}
