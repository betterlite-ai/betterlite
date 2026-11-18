use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue, text_to_bool};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::Error as _;

use super::error::CoordRedisError;
use super::settings::CoordinationRedisBlock;
use crate::proxy_request_contracts::ContractError;
use crate::proxy_request_contracts::coordination_redis::CoordinationRedisParams;

const OS_ENVIRON_PREFIX: &str = "os.environ/";

// litellm's block resolver calls get_secret, so a deployment with a secret manager resolves manager and OIDC references too.
#[async_trait]
pub trait CoordSecretLookup: Send + Sync {
    async fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
    ) -> Result<Option<SecretValue>, SecretLookupError>;
}

#[async_trait]
impl CoordSecretLookup for SecretLookup {
    async fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
    ) -> Result<Option<SecretValue>, SecretLookupError> {
        SecretLookup::get_secret(self, env, name, None)
    }
}

// A deployment with no secret manager wired reads references from its environment alone, as the oracle's process does.
pub struct EnvironmentSecrets;

#[async_trait]
impl CoordSecretLookup for EnvironmentSecrets {
    async fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
    ) -> Result<Option<SecretValue>, SecretLookupError> {
        let stripped = name.strip_prefix(OS_ENVIRON_PREFIX).unwrap_or(name);
        Ok(env.get(stripped).map(|text| {
            text_to_bool(text).map_or_else(
                || SecretValue::Text(SecretString::from(text.as_str())),
                SecretValue::Bool,
            )
        }))
    }
}

pub struct ResolveContext<'a> {
    pub secrets: &'a dyn CoordSecretLookup,
    pub env: &'a BTreeMap<String, String>,
}

// Resolves the block and validates it; a credential moves straight into its SecretString field, so no JSON value holds one.
pub async fn resolve_params(
    block: &CoordinationRedisBlock,
    context: &ResolveContext<'_>,
) -> Result<CoordinationRedisParams, CoordRedisError> {
    if !block.value.is_object() {
        return Err(CoordRedisError::NotMapping);
    }
    let mut resolved = block.value.clone();
    let password = take_secret(&mut resolved, "password", context).await?;
    let url = take_secret(&mut resolved, "url", context).await?;
    let sentinel_password = take_secret(&mut resolved, "sentinel_password", context).await?;
    resolve_refs(&mut resolved, context).await?;
    let mut params = CoordinationRedisParams::from_value(&resolved)?;
    params.password = password;
    params.url = url;
    params.sentinel_password = sentinel_password;
    Ok(params)
}

// The three fields litellm declares as a secret: their resolved text never becomes an ordinary JSON string.
async fn take_secret(
    block: &mut JsonValue,
    name: &str,
    context: &ResolveContext<'_>,
) -> Result<Option<SecretString>, CoordRedisError> {
    let JsonValue::Object(fields) = block else {
        return Ok(None);
    };
    let Some(value) = fields.remove(name) else {
        return Ok(None);
    };
    let JsonValue::String(text) = &value else {
        return secret_value(name, &value);
    };
    if !text.starts_with(OS_ENVIRON_PREFIX) {
        return secret_value(name, &value);
    }
    match context.secrets.get_secret(context.env, text).await? {
        None => Ok(None),
        Some(SecretValue::Text(secret)) => Ok(Some(secret)),
        Some(SecretValue::Bool(flag)) => Err(not_text_source(
            name,
            &format!("invalid type: {flag}, expected a string"),
        )),
    }
}

fn secret_value(name: &str, value: &JsonValue) -> Result<Option<SecretString>, CoordRedisError> {
    serde_json::from_value(value.clone())
        .map_err(|source| not_text_source(name, &source.to_string()))
}

fn not_text_source(name: &str, message: &str) -> CoordRedisError {
    CoordRedisError::Params(ContractError::from(serde_json::Error::custom(format!(
        "{name}: {message}"
    ))))
}

// A value without an "os.environ/" prefix is left alone, even when it looks like a reference.
async fn resolve_refs(
    block: &mut JsonValue,
    context: &ResolveContext<'_>,
) -> Result<(), CoordRedisError> {
    let JsonValue::Object(fields) = block else {
        return Ok(());
    };
    let mut changes = Vec::new();
    for (field, value) in fields.iter() {
        let JsonValue::String(reference) = value else {
            continue;
        };
        if !reference.starts_with(OS_ENVIRON_PREFIX) {
            continue;
        }
        let secret = context.secrets.get_secret(context.env, reference).await?;
        changes.push((field.clone(), reference_value(field, reference, secret)?));
    }
    for (field, value) in changes {
        fields.insert(field, value);
    }
    Ok(())
}

fn reference_value(
    field: &str,
    reference: &str,
    secret: Option<SecretValue>,
) -> Result<JsonValue, CoordRedisError> {
    match secret {
        None => Ok(JsonValue::Null),
        Some(SecretValue::Bool(flag)) => Ok(JsonValue::Bool(flag)),
        Some(SecretValue::Text(text)) => typed_text(field, reference, text.expose_secret()),
    }
}

// get_secret returns text for anything but "true"/"false", so the model's own field type decides the rest.
fn typed_text(field: &str, reference: &str, text: &str) -> Result<JsonValue, CoordRedisError> {
    let name = reference
        .strip_prefix(OS_ENVIRON_PREFIX)
        .unwrap_or(reference);
    match field {
        "port" => {
            let port = betterlite_env::number::<i64>(name, text)?;
            Ok(port.map_or(JsonValue::Null, |value| {
                JsonValue::Number(serde_json::Number::from(value))
            }))
        }
        "ssl" | "aws_iam_auth" | "aws_iam_serverless" => {
            let flag = betterlite_env::flag(name, text)?;
            Ok(flag.map_or(JsonValue::Null, JsonValue::Bool))
        }
        _ => Ok(JsonValue::String(text.to_owned())),
    }
}
