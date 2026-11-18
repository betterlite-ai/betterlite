use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue, text_to_bool};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::CoordRedisError;

const OS_ENVIRON_PREFIX: &str = "os.environ/";

// litellm's block resolver calls get_secret, so a deployment with a secret manager resolves manager and OIDC references too.
pub trait CoordSecretLookup: Sync {
    fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
    ) -> Result<Option<SecretValue>, SecretLookupError>;
}

impl CoordSecretLookup for SecretLookup {
    fn get_secret(
        &self,
        env: &BTreeMap<String, String>,
        name: &str,
    ) -> Result<Option<SecretValue>, SecretLookupError> {
        SecretLookup::get_secret(self, env, name, None)
    }
}

// A deployment with no secret manager wired reads references from its environment alone, as the oracle's process does.
pub struct EnvironmentSecrets;

impl CoordSecretLookup for EnvironmentSecrets {
    fn get_secret(
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

// A value without an "os.environ/" prefix is left alone, even when it looks like a reference.
pub fn resolve_env_refs(
    raw: &JsonValue,
    context: &ResolveContext<'_>,
) -> Result<JsonValue, CoordRedisError> {
    let JsonValue::Object(fields) = raw else {
        return Err(CoordRedisError::NotMapping);
    };
    let mut resolved = fields.clone();
    for (field, value) in fields {
        let JsonValue::String(reference) = value else {
            continue;
        };
        if !reference.starts_with(OS_ENVIRON_PREFIX) {
            continue;
        }
        let secret = context.secrets.get_secret(context.env, reference)?;
        resolved.insert(field.clone(), reference_value(field, reference, secret)?);
    }
    Ok(JsonValue::Object(resolved))
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
