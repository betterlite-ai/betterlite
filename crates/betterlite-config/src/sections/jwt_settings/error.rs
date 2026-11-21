use std::fmt;

use betterlite_env::EnvError;
use serde::Serialize;
use thiserror::Error;

use super::validator::{LoadFailure, ReturnAnnotation};

// LiteLLM_JWTAuth.__annotations__ in declaration order, the list LiteLLM prints after an unknown key
pub const ALLOWED_KEYS: [&str; 43] = [
    "admin_jwt_scope",
    "admin_allowed_routes",
    "team_id_jwt_field",
    "team_id_upsert",
    "team_ids_jwt_field",
    "upsert_sso_user_to_team",
    "team_allowed_routes",
    "team_id_default",
    "team_alias_jwt_field",
    "org_id_jwt_field",
    "org_alias_jwt_field",
    "user_id_jwt_field",
    "user_email_jwt_field",
    "user_allowed_email_domain",
    "user_roles_jwt_field",
    "user_allowed_roles",
    "user_id_upsert",
    "end_user_id_jwt_field",
    "agent_id_jwt_field",
    "mcp_client_id_jwt_field",
    "public_key_ttl",
    "public_key_stale_ttl",
    "public_allowed_routes",
    "enforce_rbac",
    "roles_jwt_field",
    "role_mappings",
    "object_id_jwt_field",
    "scope_mappings",
    "enforce_scope_based_access",
    "enforce_team_based_model_access",
    "custom_validate",
    "jwt_litellm_role_map",
    "sync_user_role_and_teams",
    "oidc_userinfo_endpoint",
    "oidc_userinfo_enabled",
    "oidc_userinfo_cache_ttl",
    "virtual_key_claim_field",
    "virtual_key_mapping_cache_ttl",
    "unregistered_jwt_client_behavior",
    "routing_overrides",
    "team_claim_fallback",
    "fallback_to_db_teams",
    "issuers",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Step {
    Key(String),
    Index(usize),
}

// the path pydantic reports for a rejected value, such as issuers.0.audience
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Location(pub Vec<Step>);

impl Location {
    #[must_use]
    pub fn key(&self, name: &str) -> Self {
        let mut steps = self.0.clone();
        steps.push(Step::Key(name.to_owned()));
        Self(steps)
    }

    #[must_use]
    pub fn index(&self, position: usize) -> Self {
        let mut steps = self.0.clone();
        steps.push(Step::Index(position));
        Self(steps)
    }
}

impl From<&str> for Location {
    fn from(name: &str) -> Self {
        Self(vec![Step::Key(name.to_owned())])
    }
}

impl fmt::Display for Location {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, step) in self.0.iter().enumerate() {
            if position > 0 {
                formatter.write_str(".")?;
            }
            match step {
                Step::Key(name) => formatter.write_str(name)?,
                Step::Index(index) => write!(formatter, "{index}")?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum JwtSettingsError {
    #[error("Invalid arguments provided: {}. Allowed arguments are: {}.", .invalid.join(", "), ALLOWED_KEYS.join(", "))]
    UnknownKeys { invalid: Vec<String> },
    #[error("user_allowed_roles must be provided if user_roles_jwt_field is set.")]
    RolesWithoutPair,
    #[error(
        "if object_id_jwt_field is set, role_mappings must also be set. Needed to infer if the caller is a user or team."
    )]
    ObjectIdWithoutRoleMappings,
    // LiteLLM's text names the reverse of the condition that raises it
    #[error("scope_mappings must be set if enforce_scope_based_access is true.")]
    ScopeMappingsWithoutEnforcement,
    #[error("Custom validator must be annotated to return Literal[True], got {annotation}")]
    ValidatorAnnotation { annotation: ReturnAnnotation },
    #[error(transparent)]
    ValidatorLoad {
        #[from]
        source: LoadFailure,
    },
    #[error("invalid value at {location}")]
    Invalid { location: Location },
    #[error("JWT issuer {issuer} must configure audience or set disable_audience_validation=True")]
    AudienceMissing { location: Location, issuer: String },
    #[error(
        "JWT issuer {issuer} cannot set audience and disable_audience_validation=True together"
    )]
    AudienceConflict { location: Location, issuer: String },
    #[error("environment variable {variable} for {field} is not set")]
    MissingEnvironment {
        variable: String,
        field: &'static str,
    },
    #[error(transparent)]
    Environment {
        #[from]
        source: EnvError,
    },
}
