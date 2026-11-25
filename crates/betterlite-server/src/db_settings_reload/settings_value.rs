use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::settings::coerce_bool::coerce_bool;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::SettingsReloadError;
use super::ports::{SemanticFilterConfig, SettingsRows};

// LiteLLM decodes a JSON string param_value once; every other shape fails at the .get it does next.
pub(crate) async fn litellm_settings_object<Db>(
    db: &Db,
) -> Result<Option<JsonMap>, SettingsReloadError>
where
    Db: SettingsRows + ?Sized,
{
    let Some(stored) = db.litellm_settings().await? else {
        return Ok(None);
    };
    match stored {
        JsonValue::Null => Ok(None),
        JsonValue::Object(settings) => Ok(Some(settings)),
        JsonValue::String(text) => decode_settings(&text).map(Some),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            Err(SettingsReloadError::LitellmSettingsNotAnObject)
        }
    }
}

fn decode_settings(text: &str) -> Result<JsonMap, SettingsReloadError> {
    let decoded: JsonValue = serde_json::from_str(text)?;
    match decoded {
        JsonValue::Object(settings) => Ok(settings),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Err(SettingsReloadError::LitellmSettingsNotAnObject),
    }
}

// coerce_bool then bool(), as the reload does: the stored flag may be any truthy value.
pub(crate) fn enabled_flag(config: &JsonMap) -> bool {
    let stored = config
        .get("enabled")
        .map_or(JsonValue::Bool(true), |value| coerce_bool(value.clone()));
    matches!(stored, JsonValue::Bool(true))
}

// A stored filter config must be a mapping; null is the same as no key at all.
pub(crate) fn semantic_config(stored: &JsonValue) -> Result<Option<&JsonMap>, SettingsReloadError> {
    if let JsonValue::Object(config) = stored {
        return Ok(Some(config));
    }
    if stored.is_null() {
        return Ok(None);
    }
    Err(SettingsReloadError::SemanticFilterNotAnObject)
}

// SemanticToolFilterConfig: a declared field of another type is an error naming it, a missing one keeps the hook's default.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StoredSemanticFilter {
    enabled: Option<StoredFlag>,
    embedding_model: Option<StoredText>,
    top_k: Option<StoredInteger>,
    similarity_threshold: Option<StoredFloat>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredFlag {
    Bool(bool),
    Wrong(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredText {
    Text(String),
    Wrong(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredInteger {
    Integer(i64),
    Wrong(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredFloat {
    Float(f64),
    Wrong(IgnoredAny),
}

pub(crate) fn semantic_filter_config(
    config: &JsonMap,
) -> Result<SemanticFilterConfig, SettingsReloadError> {
    let stored: StoredSemanticFilter = serde_json::from_value(JsonValue::Object(config.clone()))?;
    let enabled = match stored.enabled {
        None => false,
        Some(StoredFlag::Bool(flag)) => flag,
        Some(StoredFlag::Wrong(_)) => {
            return Err(SettingsReloadError::SemanticFilterEnabledNotBool);
        }
    };
    let embedding_model = match stored.embedding_model {
        None => None,
        Some(StoredText::Text(model)) => Some(model),
        Some(StoredText::Wrong(_)) => {
            return Err(SettingsReloadError::SemanticFilterEmbeddingModelNotString);
        }
    };
    let top_k = match stored.top_k {
        None => None,
        Some(StoredInteger::Integer(count)) => Some(count),
        Some(StoredInteger::Wrong(_)) => {
            return Err(SettingsReloadError::SemanticFilterTopKNotInteger);
        }
    };
    let similarity_threshold = match stored.similarity_threshold {
        None => None,
        Some(StoredFloat::Float(value)) => Some(value),
        Some(StoredFloat::Wrong(_)) => {
            return Err(SettingsReloadError::SemanticFilterSimilarityThresholdNotNumber);
        }
    };
    Ok(SemanticFilterConfig {
        enabled,
        embedding_model,
        top_k,
        similarity_threshold,
    })
}
