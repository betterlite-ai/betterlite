pub mod construct;
pub mod error;
pub mod groups;
pub mod records;
mod tests;
pub mod validator;

use serde::{Deserialize, Serialize};

use self::construct::{Context, RawJwtAuth, construct};
use self::error::JwtSettingsError;
use self::groups::{
    ClaimFields, KeyFields, RbacFields, RouteFields, RoutingFields, ScopeFields, SyncFields,
    TeamFields, UserinfoFields, VirtualKeyFields,
};
use self::records::{IssuerConfig, UnregisteredClientBehavior};
use self::validator::UnregisteredLoader;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const PREFIX: &str = "general_settings.litellm_jwtauth";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    litellm_jwtauth: Option<RawJwtAuth>,
}

// grouped close to LiteLLM_JWTAuth's field order, and serialized flat like its model_dump
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Settings {
    #[serde(flatten)]
    pub routes: RouteFields,
    #[serde(flatten)]
    pub teams: TeamFields,
    #[serde(flatten)]
    pub claims: ClaimFields,
    #[serde(flatten)]
    pub keys: KeyFields,
    #[serde(flatten)]
    pub rbac: RbacFields,
    #[serde(flatten)]
    pub scopes: ScopeFields,
    pub custom_validate: Option<String>,
    #[serde(flatten)]
    pub sync: SyncFields,
    #[serde(flatten)]
    pub userinfo: UserinfoFields,
    #[serde(flatten)]
    pub virtual_keys: VirtualKeyFields,
    #[serde(flatten)]
    pub routing: RoutingFields,
}

impl Settings {
    // get_issuer_config: the first exact issuer match wins, and later duplicates are never reached
    #[must_use]
    pub fn issuer_config(&self, issuer: Option<&str>) -> Option<&IssuerConfig> {
        let wanted = issuer?;
        let issuers = self.routing.issuers.as_ref()?;
        issuers.iter().find(|config| config.issuer == wanted)
    }

    // is_virtual_key_mapping_configured: a global or any issuer claim field, '' included
    #[must_use]
    pub fn mapping_configured(&self) -> bool {
        self.virtual_keys.virtual_key_claim_field.is_some()
            || self
                .routing
                .issuers
                .iter()
                .flatten()
                .any(|config| config.virtual_key_claim_field.is_some())
    }

    #[must_use]
    pub fn claim_field(&self, issuer: Option<&str>) -> Option<&str> {
        self.issuer_config(issuer)
            .and_then(|config| config.virtual_key_claim_field.as_deref())
            .or(self.virtual_keys.virtual_key_claim_field.as_deref())
    }

    // get_unregistered_jwt_client_behavior: a matched issuer's policy, else the global one
    #[must_use]
    pub fn client_behavior(&self, issuer: Option<&str>) -> UnregisteredClientBehavior {
        self.issuer_config(issuer)
            .and_then(|config| config.unregistered_client)
            .unwrap_or(self.virtual_keys.unregistered_client)
    }
}

impl Section for Raw {
    type Settings = Settings;

    // the config load has no validator loader, so a configured custom_validate stops it until B054 adds one
    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(configured) = self.general_settings.litellm_jwtauth else {
            return Ok(Settings::default());
        };
        let context = Context {
            loader: &UnregisteredLoader,
            config_file_path: None,
            lookup,
        };
        construct(configured, &context).map_err(config_error)
    }
}

fn config_error(error: JwtSettingsError) -> ConfigError {
    tracing::error!(%error, "{PREFIX} is invalid");
    let field = match error {
        JwtSettingsError::UnknownKeys { invalid } => {
            let paths = invalid.iter().map(|key| format!("{PREFIX}.{key}"));
            return ConfigError::Unsupported {
                paths: paths.collect(),
            };
        }
        JwtSettingsError::MissingEnvironment { variable, field } => {
            return ConfigError::MissingEnvironment {
                variable,
                path: format!("{PREFIX}.{field}"),
            };
        }
        JwtSettingsError::Environment { source } => return ConfigError::Environment { source },
        JwtSettingsError::RolesWithoutPair => String::from("user_allowed_roles"),
        JwtSettingsError::ObjectIdWithoutRoleMappings => String::from("role_mappings"),
        JwtSettingsError::ScopesNotEnforced => String::from("scope_mappings"),
        JwtSettingsError::ValidatorAnnotation { .. } | JwtSettingsError::ValidatorLoad { .. } => {
            String::from("custom_validate")
        }
        JwtSettingsError::Encode { .. } => {
            return ConfigError::InvalidScalar {
                path: PREFIX.to_owned(),
                value: false,
            };
        }
        JwtSettingsError::Invalid { location }
        | JwtSettingsError::AudienceMissing { location, .. }
        | JwtSettingsError::AudienceConflict { location, .. } => location.dotted(),
    };
    ConfigError::InvalidScalar {
        path: format!("{PREFIX}.{field}"),
        value: false,
    }
}
