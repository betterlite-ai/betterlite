use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::controls::multipart::DEFAULT_MAX_RECURSE_DEPTH;
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};
use secrecy::ExposeSecret as _;

use super::error::OverlayError;

const ENVIRONMENT_PREFIX: &str = "os.environ/";

pub struct Resolver<'resolve> {
    pub lookup: &'resolve SecretLookup,
    pub env: &'resolve BTreeMap<String, String>,
}

impl Resolver<'_> {
    // _check_for_os_environ_vars: a mapping past the depth limit keeps its references, and a missing value becomes null
    pub fn resolve_entries<'value, Values>(
        &self,
        values: Values,
        depth: usize,
    ) -> Result<(), OverlayError>
    where
        Values: IntoIterator<Item = &'value mut JsonValue>,
    {
        if depth > DEFAULT_MAX_RECURSE_DEPTH {
            tracing::warn!(
                "Maximum recursion depth ({DEFAULT_MAX_RECURSE_DEPTH}) reached while processing config."
            );
            return Ok(());
        }
        for value in values {
            self.resolve_value(value, depth)?;
        }
        Ok(())
    }

    // _resolved_config_value: mappings recurse, mappings inside lists recurse, and other list items stay as written
    fn resolve_value(&self, value: &mut JsonValue, depth: usize) -> Result<(), OverlayError> {
        let nested = depth.saturating_add(1);
        match value {
            JsonValue::Object(fields) => self.resolve_entries(fields.values_mut(), nested),
            JsonValue::Array(items) => {
                let mappings = items.iter_mut().filter_map(JsonValue::as_object_mut);
                for fields in mappings {
                    self.resolve_entries(fields.values_mut(), nested)?;
                }
                Ok(())
            }
            JsonValue::String(text) if text.starts_with(ENVIRONMENT_PREFIX) => {
                let secret = self.lookup.get_secret(self.env, text, None)?;
                *value = match secret {
                    Some(SecretValue::Text(resolved)) => {
                        JsonValue::String(resolved.expose_secret().to_owned())
                    }
                    Some(SecretValue::Bool(flag)) => JsonValue::Bool(flag),
                    None => JsonValue::Null,
                };
                Ok(())
            }
            JsonValue::String(_) | JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {
                Ok(())
            }
        }
    }
}
