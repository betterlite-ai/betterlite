use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use thiserror::Error;

use crate::coord_redis_boot::env_refs::{ResolveContext, resolve_params};
use crate::coord_redis_boot::error::CoordRedisError;
use crate::coord_redis_boot::settings::CoordinationRedisBlock;
use crate::proxy_request_contracts::coordination_redis::CoordinationRedisParams;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error(transparent)]
    InvalidFields(#[from] InvalidFields),
    #[error(
        "coordination_redis needs a connection target: set one of host, url, startup_nodes, or sentinel_nodes"
    )]
    MissingTarget,
    #[error(transparent)]
    Resolution(#[from] CoordRedisError),
}

// litellm renders the sorted field names as a Python list, so the message carries their repr rather than their names.
#[derive(Debug, Error)]
#[error("Invalid coordination_redis settings for fields: {0}")]
pub struct InvalidFields(pub String);

// litellm's _validated_params: resolve os.environ references the way startup does, then require a connection target.
pub async fn validated_params(
    settings: &JsonValue,
    context: &ResolveContext<'_>,
) -> Result<CoordinationRedisParams, ValidationError> {
    let block = CoordinationRedisBlock {
        value: settings.clone(),
    };
    match resolve_params(&block, context).await {
        Ok(params) if params.has_connection_target() => Ok(params),
        Ok(_) => Err(ValidationError::MissingTarget),
        Err(error) => Err(failure(settings, context, error).await),
    }
}

// A value the caller sent, or the environment value it names, is a 400 with the field named; a lookup failure stays a server error.
async fn failure(
    settings: &JsonValue,
    context: &ResolveContext<'_>,
    error: CoordRedisError,
) -> ValidationError {
    let is_input_error = matches!(
        &error,
        CoordRedisError::Params(_) | CoordRedisError::Environment(_)
    );
    if !is_input_error {
        return ValidationError::Resolution(error);
    }
    match rejected_fields(settings, context).await {
        Ok(fields) => {
            let names: Vec<String> = fields.iter().map(|field| format!("'{field}'")).collect();
            let joined = names.join(", ");
            ValidationError::InvalidFields(InvalidFields(format!("[{joined}]")))
        }
        Err(other) => ValidationError::Resolution(other),
    }
}

// litellm collects one name per pydantic error; one probe per top-level key keeps each name to the field that failed.
async fn rejected_fields(
    settings: &JsonValue,
    context: &ResolveContext<'_>,
) -> Result<Vec<String>, CoordRedisError> {
    let JsonValue::Object(members) = settings else {
        return Ok(Vec::new());
    };
    let mut rejected = BTreeSet::new();
    for (name, value) in members {
        let probe = JsonValue::Object(std::iter::once((name.clone(), value.clone())).collect());
        let block = CoordinationRedisBlock { value: probe };
        match resolve_params(&block, context).await {
            Ok(_) => {}
            Err(CoordRedisError::Params(_) | CoordRedisError::Environment(_)) => {
                rejected.insert(name.clone());
            }
            Err(other) => return Err(other),
        }
    }
    Ok(rejected.into_iter().collect())
}
