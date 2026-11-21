#[path = "team_limits/tests.rs"]
mod tests;

use std::fmt::Display;

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as DeserializeError, IgnoredAny};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number, resolve_string};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const TEAM_PARAMS_PATH: &str = "litellm_settings.default_team_params";
pub const INTERNAL_USER_PARAMS_PATH: &str = "litellm_settings.default_internal_user_params";
pub const AUTO_ADD_SWITCH_PATH: &str = "general_settings.disable_auto_add_proxy_admin_to_teams";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    default_team_params: Option<JsonValue>,
    default_internal_user_params: Option<JsonValue>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(rename = "disable_auto_add_proxy_admin_to_teams")]
    disable_admin_auto_add: Option<RawSwitch>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSwitch {
    Bool(bool),
    Other(IgnoredAny),
}

// LiteLLM keeps the configured dict as written: an absent key stays absent and a null value reads as an absent one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DefaultTeamParams {
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub team_member_permissions: Option<Vec<String>>,
    #[serde(default)]
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultInternalUserParams {
    pub available_teams: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub default_team_params: Option<DefaultTeamParams>,
    pub default_internal_user_params: Option<DefaultInternalUserParams>,
    pub disable_admin_auto_add: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            default_team_params: self
                .litellm_settings
                .default_team_params
                .map(|value| team_params(&value, lookup))
                .transpose()?,
            default_internal_user_params: self
                .litellm_settings
                .default_internal_user_params
                .map(|value| internal_user_params(&value))
                .transpose()?,
            // only the literal boolean disables the proxy admin; 1 and "true" leave the default on
            disable_admin_auto_add: matches!(
                self.general_settings.disable_admin_auto_add,
                Some(RawSwitch::Bool(true))
            ),
        })
    }
}

fn team_params<L>(value: &JsonValue, lookup: &L) -> Result<DefaultTeamParams, ConfigError>
where
    L: EnvironmentLookup,
{
    let raw: RawTeamParams = mapping(value, TEAM_PARAMS_PATH)?;
    Ok(DefaultTeamParams {
        models: raw.models,
        max_budget: resolve_number(
            raw.max_budget,
            &format!("{TEAM_PARAMS_PATH}.max_budget"),
            lookup,
            parse_money,
        )?,
        budget_duration: optional_text(
            raw.budget_duration,
            &format!("{TEAM_PARAMS_PATH}.budget_duration"),
            lookup,
        )?,
        tpm_limit: resolve_number(
            raw.tpm_limit,
            &format!("{TEAM_PARAMS_PATH}.tpm_limit"),
            lookup,
            parse_whole,
        )?,
        rpm_limit: resolve_number(
            raw.rpm_limit,
            &format!("{TEAM_PARAMS_PATH}.rpm_limit"),
            lookup,
            parse_whole,
        )?,
        // _check_for_os_environ_vars resolves dict values only, so a list entry stays as written
        team_member_permissions: raw.team_member_permissions,
        organization_id: optional_text(
            raw.organization_id,
            &format!("{TEAM_PARAMS_PATH}.organization_id"),
            lookup,
        )?,
    })
}

#[derive(Debug, Default, Deserialize)]
struct RawTeamParams {
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    max_budget: Option<RawNumber>,
    #[serde(default)]
    budget_duration: Option<String>,
    #[serde(default)]
    tpm_limit: Option<RawNumber>,
    #[serde(default)]
    rpm_limit: Option<RawNumber>,
    #[serde(default)]
    team_member_permissions: Option<Vec<String>>,
    #[serde(default)]
    organization_id: Option<String>,
}

fn internal_user_params(value: &JsonValue) -> Result<DefaultInternalUserParams, ConfigError> {
    let raw: RawInternalUserParams = mapping(value, INTERNAL_USER_PARAMS_PATH)?;
    Ok(DefaultInternalUserParams {
        available_teams: raw.available_teams,
    })
}

// the dict carries other defaults beside the one this task reads, so only its own key is typed
#[derive(Debug, Default, Deserialize)]
struct RawInternalUserParams {
    #[serde(default)]
    available_teams: Option<Vec<String>>,
}

fn mapping<T>(value: &JsonValue, path: &str) -> Result<T, ConfigError>
where
    T: DeserializeOwned,
{
    let JsonValue::Object(_) = value else {
        return Err(yaml_error(path, "the value must be a mapping"));
    };
    let outcome: Result<T, serde_path_to_error::Error<serde_json::Error>> =
        serde_path_to_error::deserialize(value.clone());
    outcome.map_err(|error| {
        let field = error.path().to_string();
        if field.is_empty() {
            return yaml_error(path, error.inner().to_string());
        }
        yaml_error(&format!("{path}.{field}"), error.inner().to_string())
    })
}

// an os.environ/ reference with an empty value counts as unset; a written value is kept as given
fn optional_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let from_environment = text.starts_with("os.environ/");
    let resolved = resolve_string(text, path, lookup)?;
    if from_environment && resolved.is_empty() {
        return Ok(None);
    }
    Ok(Some(resolved))
}

fn parse_whole(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

fn yaml_error<M>(path: &str, message: M) -> ConfigError
where
    M: Display,
{
    ConfigError::Yaml {
        source: <serde_saphyr::Error as DeserializeError>::custom(format!("{path}: {message}")),
    }
}
