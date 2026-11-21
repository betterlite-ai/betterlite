use std::path::Path;

use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use super::Settings;
use super::error::{ALLOWED_KEYS, JwtSettingsError, Location, Step};
use super::groups::{KeyFields, RoutingFields};
use super::records::IssuerConfig;
use super::validator::{LoadFailure, ReturnAnnotation, ValidatorLoader, ValidatorRequest};
use crate::loader::EnvironmentLookup;
use crate::sections::guardrail_identity::item::GuardrailValue;

// litellm_jwtauth as written, so LiteLLM's checks run before any field is typed
pub type RawJwtAuth = IndexMap<String, GuardrailValue>;

const ENV_PREFIX: &str = "os.environ/";

const FLAGS: [&str; 10] = [
    "team_id_upsert",
    "upsert_sso_user_to_team",
    "user_id_upsert",
    "enforce_rbac",
    "enforce_scope_based_access",
    "enforce_team_based_model_access",
    "sync_user_role_and_teams",
    "oidc_userinfo_enabled",
    "team_claim_fallback",
    "fallback_to_db_teams",
];

const NUMBERS: [&str; 4] = [
    "public_key_ttl",
    "public_key_stale_ttl",
    "oidc_userinfo_cache_ttl",
    "virtual_key_mapping_cache_ttl",
];

// ScopeMapping, JWTRoutingOverride and JWTIssuerConfig forbid extra keys; the role maps drop them
const FORBIDDING: [&str; 3] = ["scope_mappings", "routing_overrides", "issuers"];

#[derive(Clone, Copy)]
pub struct Context<'context> {
    pub loader: &'context dyn ValidatorLoader,
    pub config_file_path: Option<&'context Path>,
    pub lookup: &'context dyn EnvironmentLookup,
}

// proxy_server resolves references first, then LiteLLM_JWTAuth.__init__ checks the validator, unknown keys and pairs
pub fn construct(mut raw: RawJwtAuth, context: &Context<'_>) -> Result<Settings, JwtSettingsError> {
    for (field, value) in &mut raw {
        resolve(field, value, context.lookup)?;
    }
    if let Some(alias) = raw.shift_remove("jwt_client_id_field") {
        raw.entry("virtual_key_claim_field".to_owned())
            .or_insert(alias);
    }
    let custom_validate = validator(raw.shift_remove("custom_validate"), context)?;
    let mut invalid: Vec<String> = raw
        .keys()
        .filter(|key| !ALLOWED_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    if !invalid.is_empty() {
        invalid.sort();
        return Err(JwtSettingsError::UnknownKeys { invalid });
    }
    pairs(&raw)?;
    let ordered: IndexMap<&str, GuardrailValue> = ALLOWED_KEYS
        .into_iter()
        .filter_map(|key| raw.shift_remove(key).map(|value| (key, value)))
        .filter(|(_key, value)| *value != GuardrailValue::Null)
        .collect();
    settings(&serde_json::to_vec(&ordered)?, custom_validate)
}

// LiteLLM_JWTAuth.__init__'s checks on raw values, where an explicit null counts as unset
fn pairs(raw: &RawJwtAuth) -> Result<(), JwtSettingsError> {
    let given = |key: &str| {
        raw.get(key)
            .is_some_and(|value| *value != GuardrailValue::Null)
    };
    if given("user_roles_jwt_field") != given("user_allowed_roles") {
        return Err(JwtSettingsError::RolesWithoutPair);
    }
    if given("object_id_jwt_field") && !given("role_mappings") {
        return Err(JwtSettingsError::ObjectIdWithoutRoleMappings);
    }
    // a value of the wrong type passes here, and its type check below rejects it
    let enforced = raw.get("enforce_scope_based_access").is_some_and(|value| {
        !matches!(value, GuardrailValue::Null | GuardrailValue::Boolean(false))
    });
    if given("scope_mappings") && !enforced {
        return Err(JwtSettingsError::ScopesNotEnforced);
    }
    Ok(())
}

// pydantic checks the fields in annotation order, so the groups decode in that order
fn settings(bytes: &[u8], custom_validate: Option<String>) -> Result<Settings, JwtSettingsError> {
    let routes = decode(bytes)?;
    let teams = decode(bytes)?;
    let claims = decode(bytes)?;
    let keys: KeyFields = decode(bytes)?;
    if keys.public_key_stale_ttl < 0.0 {
        return Err(JwtSettingsError::Invalid {
            location: Location(vec![Step::Key("public_key_stale_ttl".to_owned())]),
        });
    }
    let rbac = decode(bytes)?;
    let scopes = decode(bytes)?;
    let sync = decode(bytes)?;
    let userinfo = decode(bytes)?;
    let virtual_keys = decode(bytes)?;
    let routing: RoutingFields = decode(bytes)?;
    for (index, issuer) in routing.issuers.iter().flatten().enumerate() {
        audience(
            issuer,
            Location(vec![Step::Key("issuers".to_owned()), Step::Index(index)]),
        )?;
    }
    Ok(Settings {
        routes,
        teams,
        claims,
        keys,
        rbac,
        scopes,
        custom_validate,
        sync,
        userinfo,
        virtual_keys,
        routing,
    })
}

// each group takes its own keys from the mapping, and serde_ignored names the record keys no field took
fn decode<T>(bytes: &[u8]) -> Result<T, JwtSettingsError>
where
    T: DeserializeOwned,
{
    let mut ignored = Vec::new();
    let mut record = |path: serde_ignored::Path<'_>| ignored.push(path.to_string());
    let mut json = serde_json::Deserializer::from_slice(bytes);
    let tracked = serde_ignored::Deserializer::new(&mut json, &mut record);
    let group =
        serde_path_to_error::deserialize(tracked).map_err(|error| JwtSettingsError::Invalid {
            location: Location::decoded(&error),
        })?;
    for path in &ignored {
        extra(path)?;
    }
    Ok(group)
}

// a top-level key belongs to another group; a key inside a record is an error or a logged drop
fn extra(path: &str) -> Result<(), JwtSettingsError> {
    // serde_ignored writes ? for the Option around each list
    let steps: Vec<&str> = path.split('.').filter(|step| *step != "?").collect();
    if steps.len() < 3 {
        return Ok(());
    }
    let location = Location(
        steps
            .iter()
            .map(|step| {
                step.parse::<usize>()
                    .map_or_else(|_text| Step::Key((*step).to_owned()), Step::Index)
            })
            .collect(),
    );
    if FORBIDDING.iter().any(|field| steps.first() == Some(field)) {
        return Err(JwtSettingsError::Invalid { location });
    }
    tracing::warn!(key = %location.dotted(), "general_settings.litellm_jwtauth ignores this key");
    Ok(())
}

// JWTIssuerConfig's after-validator runs once its fields are valid; an empty audience still counts as set
fn audience(issuer: &IssuerConfig, location: Location) -> Result<(), JwtSettingsError> {
    match (
        issuer.audience.is_some(),
        issuer.disable_audience_validation,
    ) {
        (false, false) => Err(JwtSettingsError::AudienceMissing {
            location,
            issuer: issuer.issuer.clone(),
        }),
        (true, true) => Err(JwtSettingsError::AudienceConflict {
            location,
            issuer: issuer.issuer.clone(),
        }),
        (false, true) | (true, false) => Ok(()),
    }
}

// a top-level os.environ/ value becomes the field's type through betterlite-env, and an empty one counts as unset
fn resolve(
    field: &str,
    value: &mut GuardrailValue,
    lookup: &dyn EnvironmentLookup,
) -> Result<(), JwtSettingsError> {
    let GuardrailValue::Text(text) = value else {
        return Ok(());
    };
    let Some(variable) = text.strip_prefix(ENV_PREFIX).map(str::to_owned) else {
        return Ok(());
    };
    let resolved =
        lookup
            .lookup(&variable)
            .ok_or_else(|| JwtSettingsError::MissingEnvironment {
                variable: variable.clone(),
                field: field.to_owned(),
            })?;
    let typed = if FLAGS.contains(&field) {
        betterlite_env::flag(&variable, &resolved)?.map(GuardrailValue::Boolean)
    } else if NUMBERS.contains(&field) {
        betterlite_env::number::<f64>(&variable, &resolved)?.map(GuardrailValue::Decimal)
    } else {
        (!resolved.is_empty()).then_some(GuardrailValue::Text(resolved))
    };
    *value = typed.unwrap_or(GuardrailValue::Null);
    Ok(())
}

// get_instance_fn refuses a remote module without the config file, and the hint must be Literal[True]
fn validator(
    field: Option<GuardrailValue>,
    context: &Context<'_>,
) -> Result<Option<String>, JwtSettingsError> {
    let module = match field {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Text(text)) => text,
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => {
            return Err(JwtSettingsError::Invalid {
                location: Location(vec![Step::Key("custom_validate".to_owned())]),
            });
        }
    };
    let remote = module.starts_with("s3://") || module.starts_with("gcs://");
    if remote && context.config_file_path.is_none() {
        return Err(JwtSettingsError::ValidatorLoad {
            source: LoadFailure::RemoteWithoutConfigFile,
        });
    }
    let request = ValidatorRequest {
        module: &module,
        config_file_path: context.config_file_path,
    };
    match context.loader.load(&request)? {
        ReturnAnnotation::LiteralTrue => Ok(Some(module)),
        ReturnAnnotation::Other(annotation) => {
            Err(JwtSettingsError::ValidatorAnnotation { annotation })
        }
    }
}
