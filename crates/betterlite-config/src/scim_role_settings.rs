mod tests;
use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use serde::Deserialize;
const ADMIN_GROUP_PATH: &str = "litellm_settings.scim_admin_group";
const INTERNAL_PARAMS_PATH: &str = "general_settings.default_internal_user_params";
const DEFAULT_ROLE_PATH: &str = "general_settings.default_internal_user_params.user_role";
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    litellm_settings: Option<LitellmSettings>,
    general_settings: Option<GeneralSettings>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LitellmSettings {
    scim_upsert_user: Option<JsonValue>,
    scim_admin_group: Option<JsonValue>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GeneralSettings {
    default_internal_user_params: Option<JsonValue>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScimUserRole {
    ProxyAdmin,
    ProxyAdminViewer,
    InternalUser,
    InternalUserViewer,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub scim_upsert_user: bool,
    pub scim_admin_group: Option<String>,
    pub default_user_role: Option<ScimUserRole>,
}
impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let litellm = self.litellm_settings.unwrap_or_default();
        let general = self.general_settings.unwrap_or_default();
        Ok(Settings {
            // bool(litellm_settings.get("scim_upsert_user", True)) keeps Python's truthiness for every present value
            scim_upsert_user: litellm.scim_upsert_user.as_ref().is_none_or(is_set),
            scim_admin_group: scim_admin_group(litellm.scim_admin_group.as_ref())?,
            default_user_role: default_user_role(general.default_internal_user_params.as_ref())?,
        })
    }
}
pub fn load_scim_role_settings<L>(source: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    Section::convert(raw, lookup)
}
// LiteLLM reads the group with litellm_settings.get("scim_admin_group") or None
fn scim_admin_group(configured: Option<&JsonValue>) -> Result<Option<String>, ConfigError> {
    let Some(value) = configured else {
        return Ok(None);
    };
    if !is_set(value) {
        return Ok(None);
    }
    match value {
        JsonValue::String(text) => Ok(Some(text.clone())),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Err(invalid_scalar(ADMIN_GROUP_PATH, value)),
    }
}
// a configured user_role supplies the fallback, and an unknown value is outside the role literal NewUserRequest validates
fn default_user_role(configured: Option<&JsonValue>) -> Result<Option<ScimUserRole>, ConfigError> {
    let Some(value) = configured else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let JsonValue::Object(params) = value else {
        return Err(invalid_scalar(INTERNAL_PARAMS_PATH, value));
    };
    let Some(role) = params.get("user_role") else {
        return Ok(None);
    };
    if role.is_null() {
        return Ok(None);
    }
    serde_json::from_value(role.clone())
        .map(Some)
        .map_err(|_error| invalid_scalar(DEFAULT_ROLE_PATH, role))
}
fn invalid_scalar(path: &str, value: &JsonValue) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: matches!(value, JsonValue::Bool(true)),
    }
}
fn is_set(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(entries) => !entries.is_empty(),
    }
}
