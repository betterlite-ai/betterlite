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
        let mut configured = self.general_settings.litellm_jwtauth;
        let teams = TeamFields::resolve(&mut configured, lookup)?;
        let orgs = OrgFields::resolve(&mut configured, lookup)?;
        let identity = IdentityFields::resolve(&mut configured, lookup)?;
        Ok(Settings {
            team_id_jwt_field: teams.id,
            team_ids_jwt_field: teams.ids,
            team_id_default: teams.default_id,
            team_alias_jwt_field: teams.alias,
            org_id_jwt_field: orgs.id,
            org_alias_jwt_field: orgs.alias,
            user_id_jwt_field: identity.user_id,
            user_email_jwt_field: identity.user_email,
            user_allowed_email_domain: identity.allowed_email_domain,
            user_id_upsert: identity.upsert,
            end_user_id_jwt_field: identity.end_user_id,
            agent_id_jwt_field: identity.agent_id,
            object_id_jwt_field: identity.object_id,
        })
    }
}

struct TeamFields {
    id: Option<String>,
    ids: Option<String>,
    default_id: Option<String>,
    alias: Option<String>,
}

impl TeamFields {
    fn resolve<L>(raw: &mut RawJwtAuth, lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Self {
            id: text_setting(raw.team_id_jwt_field.take(), "team_id_jwt_field", lookup)?,
            ids: text_setting(raw.team_ids_jwt_field.take(), "team_ids_jwt_field", lookup)?,
            default_id: text_setting(raw.team_id_default.take(), "team_id_default", lookup)?,
            alias: text_setting(
                raw.team_alias_jwt_field.take(),
                "team_alias_jwt_field",
                lookup,
            )?,
        })
    }
}

struct OrgFields {
    id: Option<String>,
    alias: Option<String>,
}

impl OrgFields {
    fn resolve<L>(raw: &mut RawJwtAuth, lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Self {
            id: text_setting(raw.org_id_jwt_field.take(), "org_id_jwt_field", lookup)?,
            alias: text_setting(
                raw.org_alias_jwt_field.take(),
                "org_alias_jwt_field",
                lookup,
            )?,
        })
    }
}

struct IdentityFields {
    user_id: Option<String>,
    user_email: Option<String>,
    allowed_email_domain: Option<String>,
    upsert: bool,
    end_user_id: Option<String>,
    agent_id: Option<String>,
    object_id: Option<String>,
}

impl IdentityFields {
    fn resolve<L>(raw: &mut RawJwtAuth, lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Self {
            user_id: text_setting(raw.user_id_jwt_field.take(), "user_id_jwt_field", lookup)?,
            user_email: text_setting(
                raw.user_email_jwt_field.take(),
                "user_email_jwt_field",
                lookup,
            )?,
            allowed_email_domain: text_setting(
                raw.user_allowed_email_domain.take(),
                "user_allowed_email_domain",
                lookup,
            )?,
            upsert: flag_setting(raw.user_id_upsert.take(), "user_id_upsert", lookup)?,
            end_user_id: text_setting(
                raw.end_user_id_jwt_field.take(),
                "end_user_id_jwt_field",
                lookup,
            )?,
            agent_id: text_setting(raw.agent_id_jwt_field.take(), "agent_id_jwt_field", lookup)?,
            object_id: text_setting(
                raw.object_id_jwt_field.take(),
                "object_id_jwt_field",
                lookup,
            )?,
        })
    }
}

// an os.environ/ reference resolves through the supplied environment, and an empty variable counts as unset
fn text_setting<L>(
    raw: Option<RawText>,
    field: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = raw else {
        return Ok(None);
    };
    let RawText::Text(text) = configured else {
        return Err(ConfigError::InvalidScalar {
            path: format!("{PREFIX}.{field}"),
            value: false,
        });
    };
    let Some(variable) = text.strip_prefix("os.environ/") else {
        return Ok(Some(text));
    };
    let Some(value) = lookup.lookup(variable) else {
        return Err(ConfigError::MissingEnvironment {
            variable: variable.to_owned(),
            path: format!("{PREFIX}.{field}"),
        });
    };
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value))
}

// a flag is a YAML bool or an os.environ/ reference read by the shared parser; anything else is a load error
fn flag_setting<L>(raw: Option<RawFlag>, field: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(configured) = raw else {
        return Ok(false);
    };
    match configured {
        RawFlag::Bool(flag) => Ok(flag),
        RawFlag::Text(text) => {
            let Some(variable) = text.strip_prefix("os.environ/") else {
                return Err(ConfigError::InvalidScalar {
                    path: format!("{PREFIX}.{field}"),
                    value: false,
                });
            };
            let Some(value) = lookup.lookup(variable) else {
                return Err(ConfigError::MissingEnvironment {
                    variable: variable.to_owned(),
                    path: format!("{PREFIX}.{field}"),
                });
            };
            Ok(betterlite_env::flag(variable, &value)?.unwrap_or(false))
        }
        RawFlag::Other(_) => Err(ConfigError::InvalidScalar {
            path: format!("{PREFIX}.{field}"),
            value: false,
        }),
    }
}
