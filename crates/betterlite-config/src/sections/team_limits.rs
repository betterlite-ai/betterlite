#[path = "team_limits/tests.rs"]
mod tests;

use std::fmt::Display;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as DeserializeError, IgnoredAny};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number, resolve_string};
use crate::sections::Section;
use crate::sections::auth_user_lookup::DefaultUserParams;
use crate::sections::settings::JsonValue;

pub const TEAM_PARAMS_PATH: &str = "litellm_settings.default_team_params";
pub const AVAILABLE_TEAMS_PATH: &str =
    "litellm_settings.default_internal_user_params.available_teams";
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
pub struct Settings {
    pub default_team_params: Option<DefaultTeamParams>,
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
    let max_budget = resolve_number(
        raw.max_budget,
        &format!("{TEAM_PARAMS_PATH}.max_budget"),
        lookup,
        parse_money,
    )?;
    let budget_duration = optional_text(
        raw.budget_duration,
        &format!("{TEAM_PARAMS_PATH}.budget_duration"),
        lookup,
    )?;
    let tpm_limit = resolve_number(
        raw.tpm_limit,
        &format!("{TEAM_PARAMS_PATH}.tpm_limit"),
        lookup,
        parse_whole,
    )?;
    let rpm_limit = resolve_number(
        raw.rpm_limit,
        &format!("{TEAM_PARAMS_PATH}.rpm_limit"),
        lookup,
        parse_whole,
    )?;
    let organization_id = optional_text(
        raw.organization_id,
        &format!("{TEAM_PARAMS_PATH}.organization_id"),
        lookup,
    )?;
    Ok(DefaultTeamParams {
        models: raw.models,
        max_budget,
        budget_duration,
        tpm_limit,
        rpm_limit,
        // _check_for_os_environ_vars resolves dict values only, so a list entry stays as written
        team_member_permissions: raw.team_member_permissions,
        organization_id,
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

// litellm.default_internal_user_params is auth_user_lookup's record; this task reads only its available_teams key
pub fn available_teams(
    params: Option<&DefaultUserParams>,
) -> Result<Option<Vec<String>>, ConfigError> {
    let Some(record) = params else {
        return Ok(None);
    };
    let Some(value) = record.values.get("available_teams") else {
        return Ok(None);
    };
    match value {
        JsonData::Null => Ok(None),
        JsonData::Array(entries) => {
            let mut teams = Vec::with_capacity(entries.len());
            for entry in entries {
                let JsonData::String(team) = entry else {
                    return Err(yaml_error(
                        AVAILABLE_TEAMS_PATH,
                        "an entry must be a string",
                    ));
                };
                teams.push(team.clone());
            }
            Ok(Some(teams))
        }
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_) => Err(
            yaml_error(AVAILABLE_TEAMS_PATH, "the value must be a list of strings"),
        ),
    }
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
