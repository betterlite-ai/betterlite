use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::models::{CyberArkConfig, HashicorpVaultConfig, OverrideKind};

// The request body after Pydantic's validation, with its non-null field values.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedOverride {
    pub config: OverrideConfig,
    pub values: JsonMap,
}

#[derive(Clone, PartialEq, Eq)]
pub enum OverrideConfig {
    HashicorpVault(HashicorpVaultConfig),
    CyberArk(CyberArkConfig),
}

#[derive(Debug, thiserror::Error)]
pub enum OverrideDecodeError {
    #[error("the config override request is not valid JSON: {0}")]
    Json(#[from] PrivateJsonError),
    #[error("the config override request must be a JSON object")]
    NotAnObject,
    #[error("invalid {kind} config override: {source}")]
    Field {
        kind: OverrideKind,
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum StoredOverrideError {
    #[error("the stored {kind} config override is valid JSON but not an object")]
    NotAnObject { kind: OverrideKind },
}

// Pydantic ignores a request field the model does not declare and rejects a value that is not a string or null.
pub fn decode_config_override(
    bytes: &[u8],
    kind: OverrideKind,
) -> Result<DecodedOverride, OverrideDecodeError> {
    let JsonValue::Object(object) = safe_json_loads(bytes)? else {
        return Err(OverrideDecodeError::NotAnObject);
    };
    let values = request_values(&object, kind);
    let config = match kind {
        OverrideKind::HashicorpVault => {
            let model = serde_path_to_error::deserialize::<_, HashicorpVaultConfig>(
                JsonValue::Object(object),
            )
            .map_err(|source| OverrideDecodeError::Field { kind, source })?;
            OverrideConfig::HashicorpVault(model)
        }
        OverrideKind::CyberArk => {
            let model =
                serde_path_to_error::deserialize::<_, CyberArkConfig>(JsonValue::Object(object))
                    .map_err(|source| OverrideDecodeError::Field { kind, source })?;
            OverrideConfig::CyberArk(model)
        }
    };
    Ok(DecodedOverride { config, values })
}

fn request_values(object: &JsonMap, kind: OverrideKind) -> JsonMap {
    kind.fields()
        .iter()
        .filter_map(|field| {
            let Some(JsonValue::String(text)) = object.get(field.name) else {
                return None;
            };
            Some((field.name.to_owned(), JsonValue::String(text.clone())))
        })
        .collect()
}

// A decoded row is either an object or LiteLLM's safe_json_loads fallback to {}; any other JSON fails at use.
pub fn decode_stored_override(
    value: &JsonValue,
    kind: OverrideKind,
) -> Result<JsonMap, StoredOverrideError> {
    let JsonValue::String(text) = value else {
        return object_or_shape_error(value.clone(), kind);
    };
    safe_json_loads(text.as_bytes()).map_or_else(
        |_| Ok(JsonMap::new()),
        |parsed| object_or_shape_error(parsed, kind),
    )
}

fn object_or_shape_error(
    value: JsonValue,
    kind: OverrideKind,
) -> Result<JsonMap, StoredOverrideError> {
    match value {
        JsonValue::Object(object) => Ok(object),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Err(StoredOverrideError::NotAnObject { kind }),
    }
}
