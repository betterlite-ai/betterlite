use std::str;

use async_trait::async_trait;
use betterlite_config::sections::settings::settings_rules::JsonValue as Value;

#[async_trait]
pub trait BucketObjectReader: Send + Sync {
    async fn read_object(&self, object_key: &str) -> Option<Vec<u8>>;
}

// LiteLLM's _parsed_config: an empty YAML document reads as an empty mapping, a YAML error as a missing object
#[must_use]
pub fn parse_bucket_object(object_key: &str, bytes: &[u8]) -> Option<Value> {
    let text = match str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            tracing::error!(object_key, %error, "bucket config object is not valid UTF-8");
            return None;
        }
    };
    match serde_saphyr::from_str::<Value>(text) {
        Ok(value) => Some(value),
        Err(error) if matches!(error, serde_saphyr::Error::Eof { .. }) => Some(Value::Null),
        Err(error) => {
            tracing::error!(object_key, %error, "Config object is not valid YAML");
            None
        }
    }
}

mod tests;
