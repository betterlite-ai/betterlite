use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const PREFIX: &str = "general_settings.litellm_jwtauth";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    litellm_jwtauth: RawJwtAuth,
}

#[derive(Debug, Default, Deserialize)]
struct RawJwtAuth {
    team_id_jwt_field: Option<RawText>,
    team_ids_jwt_field: Option<RawText>,
    team_id_default: Option<RawText>,
    team_alias_jwt_field: Option<RawText>,
    org_id_jwt_field: Option<RawText>,
    org_alias_jwt_field: Option<RawText>,
    user_id_jwt_field: Option<RawText>,
    user_email_jwt_field: Option<RawText>,
    user_allowed_email_domain: Option<RawText>,
    user_id_upsert: Option<RawFlag>,
    end_user_id_jwt_field: Option<RawText>,
    agent_id_jwt_field: Option<RawText>,
    object_id_jwt_field: Option<RawText>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawText {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Bool(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub team_id_jwt_field: Option<String>,
    pub team_ids_jwt_field: Option<String>,
    pub team_id_default: Option<String>,
    pub team_alias_jwt_field: Option<String>,
    pub org_id_jwt_field: Option<String>,
    pub org_alias_jwt_field: Option<String>,
    pub user_id_jwt_field: Option<String>,
    pub user_email_jwt_field: Option<String>,
    pub user_allowed_email_domain: Option<String>,
    pub user_id_upsert: bool,
    pub end_user_id_jwt_field: Option<String>,
    pub agent_id_jwt_field: Option<String>,
    pub object_id_jwt_field: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = self.general_settings.litellm_jwtauth;
        Ok(Settings {
            team_id_jwt_field: text(configured.team_id_jwt_field, "team_id_jwt_field", lookup)?,
            team_ids_jwt_field: text(configured.team_ids_jwt_field, "team_ids_jwt_field", lookup)?,
            team_id_default: text(configured.team_id_default, "team_id_default", lookup)?,
            team_alias_jwt_field: text(
                configured.team_alias_jwt_field,
                "team_alias_jwt_field",
                lookup,
            )?,
            org_id_jwt_field: text(configured.org_id_jwt_field, "org_id_jwt_field", lookup)?,
            org_alias_jwt_field: text(
                configured.org_alias_jwt_field,
                "org_alias_jwt_field",
                lookup,
            )?,
            user_id_jwt_field: text(configured.user_id_jwt_field, "user_id_jwt_field", lookup)?,
            user_email_jwt_field: text(
                configured.user_email_jwt_field,
                "user_email_jwt_field",
                lookup,
            )?,
            user_allowed_email_domain: text(
                configured.user_allowed_email_domain,
                "user_allowed_email_domain",
                lookup,
            )?,
            user_id_upsert: flag(configured.user_id_upsert, "user_id_upsert", lookup)?,
            end_user_id_jwt_field: text(
                configured.end_user_id_jwt_field,
                "end_user_id_jwt_field",
                lookup,
            )?,
            agent_id_jwt_field: text(configured.agent_id_jwt_field, "agent_id_jwt_field", lookup)?,
            object_id_jwt_field: text(
                configured.object_id_jwt_field,
                "object_id_jwt_field",
                lookup,
            )?,
        })
    }
}

// an os.environ/ reference resolves through the supplied environment, and an empty variable counts as unset
fn text<L>(raw: Option<RawText>, field: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = raw else {
        return Ok(None);
    };
    let RawText::Text(value) = configured else {
        return Err(invalid_scalar(field));
    };
    let Some(name) = value.strip_prefix("os.environ/") else {
        return Ok(Some(value));
    };
    let resolved = lookup
        .lookup(name)
        .ok_or_else(|| missing_env(name, field))?;
    Ok((!resolved.is_empty()).then_some(resolved))
}

// a flag is a YAML bool or an os.environ/ reference read by the shared parser; anything else is a load error
fn flag<L>(raw: Option<RawFlag>, field: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = raw else {
        return Ok(false);
    };
    if let RawFlag::Bool(value) = configured {
        return Ok(value);
    }
    let RawFlag::Text(reference) = configured else {
        return Err(invalid_scalar(field));
    };
    let Some(name) = reference.strip_prefix("os.environ/") else {
        return Err(invalid_scalar(field));
    };
    environment_flag(name, field, lookup)
}

fn environment_flag<L>(name: &str, field: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = lookup
        .lookup(name)
        .ok_or_else(|| missing_env(name, field))?;
    Ok(betterlite_env::flag(name, &value)?.unwrap_or(false))
}

fn missing_env(name: &str, field: &str) -> ConfigError {
    ConfigError::MissingEnvironment {
        variable: name.to_owned(),
        path: format!("{PREFIX}.{field}"),
    }
}

fn invalid_scalar(field: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: format!("{PREFIX}.{field}"),
        value: false,
    }
}
