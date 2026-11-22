use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_claims::claims::JwtClaims;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_role_claims::rbac::rbac_role;
use betterlite_auth::jwt_role_claims::role_map::litellm_role;
use betterlite_auth::jwt_role_claims::roles::{RoleField, is_allowed_user_role, role_claim};
use betterlite_auth::jwt_role_claims::scopes::scopes;
use betterlite_config::sections::jwt_settings::Settings;
use betterlite_config::sections::jwt_settings::records::{RoleMap, RoleMapping};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    identity: JwtAuthSettings,
    settings: RoleSettings,
    claims: TypedValue,
    projection: Projection,
}

// the LiteLLM_JWTAuth keys the role projections read; identity holds the ones B760's claims read
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleSettings {
    admin_jwt_scope: Option<String>,
    user_roles_jwt_field: Option<String>,
    user_allowed_roles: Option<Vec<String>>,
    roles_jwt_field: Option<String>,
    role_mappings: Option<Vec<RoleMapping>>,
    jwt_litellm_role_map: Option<Vec<RoleMap>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Projection {
    Scopes,
    UserRoles { default_value: Option<Vec<String>> },
    JwtRole { default_value: Option<Vec<String>> },
    IsAllowedUserRole { user_roles: Option<Vec<String>> },
    RbacRole,
    LitellmRole,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = role_settings(&input.settings);
        let claims = JwtClaims {
            claims: input.claims.clone(),
            settings: &input.identity,
            trusted_issuers: &[],
        };
        project(&input.projection, &claims, &settings)
    }
}

fn role_settings(given: &RoleSettings) -> Settings {
    let mut settings = Settings::default();
    if let Some(scope) = &given.admin_jwt_scope {
        scope.clone_into(&mut settings.routes.admin_jwt_scope);
    }
    let claims = &mut settings.claims;
    claims
        .user_roles_jwt_field
        .clone_from(&given.user_roles_jwt_field);
    claims
        .user_allowed_roles
        .clone_from(&given.user_allowed_roles);
    settings
        .rbac
        .roles_jwt_field
        .clone_from(&given.roles_jwt_field);
    settings.rbac.role_mappings.clone_from(&given.role_mappings);
    let sync = &mut settings.sync;
    sync.jwt_litellm_role_map
        .clone_from(&given.jwt_litellm_role_map);
    settings
}

fn project(projection: &Projection, claims: &JwtClaims<'_>, settings: &Settings) -> HelperResult {
    let token = &claims.claims;
    Ok(match projection {
        Projection::Scopes => json!(scopes(token).or_value_error()?),
        Projection::UserRoles { default_value } => {
            let field = RoleField::UserRoles;
            json!(role_claim(token, settings, field, default_value.as_deref()).or_value_error()?)
        }
        Projection::JwtRole { default_value } => {
            let field = RoleField::Roles;
            json!(role_claim(token, settings, field, default_value.as_deref()).or_value_error()?)
        }
        Projection::IsAllowedUserRole { user_roles } => {
            let claim = user_roles
                .as_ref()
                .map(|roles| TypedValue { json: json!(roles) });
            json!(is_allowed_user_role(claim.as_ref(), settings).or_value_error()?)
        }
        Projection::RbacRole => json!(rbac_role(claims, settings).or_value_error()?),
        Projection::LitellmRole => json!(litellm_role(token, settings).or_value_error()?),
    })
}
