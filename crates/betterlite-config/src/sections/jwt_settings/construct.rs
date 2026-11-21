use std::path::Path;

use super::Settings;
use super::build::{
    admin, claims, keys, rbac, routing, scopes, sync, teams, userinfo, virtual_keys,
};
use super::convert::{ENVIRONMENT_PREFIX, Fields};
use super::error::JwtSettingsError;
use super::raw::{RawFlag, RawJwtAuth, Supplied};
use super::validator::{LoadFailure, ReturnAnnotation, ValidatorLoader, ValidatorRequest};
use crate::loader::EnvironmentLookup;

#[derive(Clone, Copy)]
pub struct Context<'context> {
    pub loader: &'context dyn ValidatorLoader,
    pub config_file_path: Option<&'context Path>,
    pub lookup: &'context dyn EnvironmentLookup,
}

// LiteLLM_JWTAuth.__init__'s order: custom_validate, unknown keys, the paired fields, then each field's type
pub fn construct(raw: RawJwtAuth, context: &Context<'_>) -> Result<Settings, JwtSettingsError> {
    let fields = Fields {
        lookup: context.lookup,
    };
    let custom_validate = validator(raw.custom_validate, context)?;
    if !raw.unknown.is_empty() {
        return Err(JwtSettingsError::UnknownKeys {
            invalid: raw.unknown.into_keys().collect(),
        });
    }
    if text_given(&raw.claims.user_roles_jwt_field, context.lookup)
        != given(&raw.claims.user_allowed_roles)
    {
        return Err(JwtSettingsError::RolesWithoutPair);
    }
    if text_given(&raw.rbac.object_id_jwt_field, context.lookup) && !given(&raw.rbac.role_mappings)
    {
        return Err(JwtSettingsError::ObjectIdWithoutRoleMappings);
    }
    if given(&raw.scopes.scope_mappings)
        && !enforcement_truthy(&raw.scopes.enforce_scope_based_access, context.lookup)
    {
        return Err(JwtSettingsError::ScopeMappingsWithoutEnforcement);
    }
    Ok(Settings {
        admin: admin(raw.admin, fields)?,
        teams: teams(raw.teams, fields)?,
        claims: claims(raw.claims, fields)?,
        keys: keys(raw.keys, fields)?,
        rbac: rbac(raw.rbac, fields)?,
        scopes: scopes(raw.scopes, fields)?,
        custom_validate,
        sync: sync(raw.sync, fields)?,
        userinfo: userinfo(raw.userinfo, fields)?,
        virtual_keys: virtual_keys(raw.virtual_keys, raw.jwt_client_id_field, fields)?,
        routing: routing(raw.routing, fields)?,
    })
}

// get_instance_fn refuses a remote module without the config file, and the hint must be Literal[True]
fn validator(
    field: Supplied<String>,
    context: &Context<'_>,
) -> Result<Option<String>, JwtSettingsError> {
    let fields = Fields {
        lookup: context.lookup,
    };
    let Some(module) = fields.text(field, "custom_validate")? else {
        return Ok(None);
    };
    let remote = module.starts_with("s3://") || module.starts_with("gcs://");
    if remote && context.config_file_path.is_none() {
        return Err(JwtSettingsError::ValidatorLoad {
            source: LoadFailure::RemoteWithoutConfigFile,
        });
    }
    let annotation = context.loader.load(&ValidatorRequest {
        module: &module,
        config_file_path: context.config_file_path,
    })?;
    if annotation != ReturnAnnotation::LiteralTrue {
        return Err(JwtSettingsError::ValidatorAnnotation { annotation });
    }
    Ok(Some(module))
}

fn given<T>(field: &Supplied<T>) -> bool {
    match field {
        Supplied::Absent | Supplied::Null => false,
        Supplied::Value(_) | Supplied::Invalid => true,
    }
}

// the paired checks are None checks on the resolved value: an unset reference is None, any other value counts
fn text_given(field: &Supplied<String>, lookup: &dyn EnvironmentLookup) -> bool {
    match field {
        Supplied::Absent | Supplied::Null => false,
        Supplied::Invalid => true,
        Supplied::Value(text) => text
            .strip_prefix(ENVIRONMENT_PREFIX)
            .is_none_or(|variable| {
                lookup
                    .lookup(variable)
                    .is_some_and(|value| !value.is_empty())
            }),
    }
}

// LiteLLM tests the value before pydantic converts it: get_secret turns only true and false into bools
fn enforcement_truthy(field: &Supplied<RawFlag>, lookup: &dyn EnvironmentLookup) -> bool {
    match field {
        Supplied::Absent | Supplied::Null => false,
        Supplied::Invalid => true,
        Supplied::Value(RawFlag::Bool(flag)) => *flag,
        Supplied::Value(RawFlag::Text(text)) => {
            text.strip_prefix(ENVIRONMENT_PREFIX)
                .map_or(!text.is_empty(), |variable| {
                    lookup.lookup(variable).is_some_and(|value| {
                        !value.is_empty() && !value.trim().eq_ignore_ascii_case("false")
                    })
                })
        }
    }
}
