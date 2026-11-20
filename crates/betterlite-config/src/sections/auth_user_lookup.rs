mod redact;
mod resolve_json;
mod tests;

use std::collections::BTreeMap;
use std::fmt;

use betterlite_db::budget_permission_rows::JsonData;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const ROLE_PERMISSIONS_PATH: &str = "general_settings.role_permissions";
pub const DEFAULT_USER_PARAMS_PATH: &str = "litellm_settings.default_internal_user_params";
const MAX_NESTED_DEPTH: usize = 64;

// litellm/proxy/_types.py's RBAC_ROLES: the three roles general_settings.role_permissions may name
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbacRole {
    ProxyAdmin,
    Team,
    InternalUser,
}

// litellm/proxy/_types.py's RoleBasedPermissions over OIDCPermissions; extra fields are forbidden
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleBasedPermissions {
    pub role: RbacRole,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub routes: Option<Vec<String>>,
}

// litellm.default_internal_user_params after proxy_server.py's max_budget float() conversion
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DefaultUserParams {
    pub values: IndexMap<String, JsonData>,
}

impl fmt::Debug for DefaultUserParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DefaultUserParams")
            .field("values", &redact::redacted_params(&self.values))
            .finish()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub role_permissions: Option<Vec<RoleBasedPermissions>>,
    pub default_internal_user_params: Option<DefaultUserParams>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    role_permissions: Option<RawValue>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    default_internal_user_params: Option<RawValue>,
}

// The settings are JSON values, but YAML can spell a non-finite float that JSON cannot hold
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawValue {
    Integer(i64),
    Float(f64),
    Text(String),
    Boolean(bool),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
    Null,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment: &dyn EnvironmentLookup = lookup;
        let role_permissions = self
            .general_settings
            .role_permissions
            .map(|value| role_permissions(value, ROLE_PERMISSIONS_PATH, environment))
            .transpose()?
            .flatten();
        let default_internal_user_params = self
            .litellm_settings
            .default_internal_user_params
            .map(|value| user_defaults(value, environment))
            .transpose()?
            .flatten();
        let settings = Settings {
            role_permissions,
            default_internal_user_params,
        };
        log_defaults(&settings);
        Ok(settings)
    }
}

fn role_permissions(
    value: RawValue,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Vec<RoleBasedPermissions>>, ConfigError> {
    let resolved = resolve_json::resolved(value, path, lookup)?;
    if matches!(resolved, JsonData::Null) {
        return Ok(None);
    }
    let parsed = serde_json::to_value(&resolved)
        .and_then(serde_json::from_value::<Vec<RoleBasedPermissions>>)
        .map_err(|_source| invalid(path, false))?;
    Ok(Some(parsed))
}

fn user_defaults(
    value: RawValue,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<DefaultUserParams>, ConfigError> {
    let path = DEFAULT_USER_PARAMS_PATH;
    let resolved = resolve_json::resolved(value, path, lookup)?;
    match resolved {
        JsonData::Object(fields) if fields.is_empty() => Ok(None),
        JsonData::Object(fields) => Ok(Some(DefaultUserParams {
            values: budget_converted(fields, path)?,
        })),
        JsonData::Array(items) if items.is_empty() => Ok(None),
        JsonData::String(text) if text.is_empty() => Ok(None),
        JsonData::Bool(false) | JsonData::Null => Ok(None),
        JsonData::Number(number) if number.as_f64() == Some(0.0_f64) => Ok(None),
        JsonData::Bool(true) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
            Err(invalid(path, false))
        }
    }
}

// proxy_server.py converts a non-None max_budget with float(); the other fields are kept as configured
fn budget_converted(
    fields: BTreeMap<String, JsonData>,
    path: &str,
) -> Result<IndexMap<String, JsonData>, ConfigError> {
    let mut values = IndexMap::new();
    for (key, value) in fields {
        if key == "max_budget" && !matches!(value, JsonData::Null) {
            let budget_path = format!("{path}.max_budget");
            values.insert(key, budget_number(&value, &budget_path)?);
            continue;
        }
        values.insert(key, value);
    }
    Ok(values)
}

fn budget_number(value: &JsonData, path: &str) -> Result<JsonData, ConfigError> {
    let text = match value {
        JsonData::Bool(flag) => String::from(if *flag { "1" } else { "0" }),
        JsonData::Number(number) => number.to_string(),
        JsonData::String(text) => text.clone(),
        JsonData::Null | JsonData::Array(_) | JsonData::Object(_) => {
            return Err(invalid(path, false));
        }
    };
    let decimal = decimal_text(text.trim()).ok_or_else(|| invalid(path, false))?;
    let float = decimal
        .to_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid(path, false))?;
    serde_json::Number::from_f64(float)
        .map(JsonData::Number)
        .ok_or_else(|| invalid(path, false))
}

fn decimal_text(text: &str) -> Option<Decimal> {
    text.parse::<Decimal>()
        .ok()
        .or_else(|| Decimal::from_scientific(text).ok())
}

fn log_defaults(settings: &Settings) {
    let Some(defaults) = settings.default_internal_user_params.as_ref() else {
        return;
    };
    tracing::debug!(
        setting = DEFAULT_USER_PARAMS_PATH,
        value = ?defaults,
        "litellm.default_internal_user_params is set"
    );
}

fn invalid(path: &str, value: bool) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value,
    }
}
