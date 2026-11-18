use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

// a key LiteLLM keeps counting even when the deployment is not in this instance's catalog
pub const COUNTED_TOKENS_KEY: &str = "_litellm_router_usage_counted_tokens";
// track_deployment_metrics._METADATA_CHANNELS: the channels the success callback reads, in order
pub const LITELLM_METADATA_CHANNEL: &str = "litellm_metadata";
pub const METADATA_CHANNEL: &str = "metadata";
const MODEL_INFO_KEY: &str = "model_info";

// a channel or bucket value of another type is still present: LiteLLM decides with `key in bucket`
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue<T> {
    Value(T),
    Other(IgnoredAny),
}

impl<T> MetadataValue<T> {
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Other(_ignored) => None,
        }
    }
}

// every key of a bucket is kept, so a null or mistyped model_info is still a present key
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetadataBucket {
    #[serde(flatten)]
    pub fields: BTreeMap<String, MetadataField>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MetadataField {
    Counted(i64),
    Other(IgnoredAny),
}

impl MetadataBucket {
    // only a JSON integer is a token count, where a float, a bool and a string are not
    #[must_use]
    pub fn counted_tokens(&self) -> Option<i64> {
        match self.fields.get(COUNTED_TOKENS_KEY) {
            Some(MetadataField::Counted(value)) => Some(*value),
            Some(MetadataField::Other(_ignored)) | None => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CallbackChannels {
    pub litellm_metadata: Option<MetadataValue<MetadataBucket>>,
    pub metadata: Option<MetadataValue<MetadataBucket>>,
}

impl CallbackChannels {
    #[must_use]
    pub fn named_buckets(&self) -> [(&'static str, Option<&MetadataBucket>); 2] {
        [
            (
                LITELLM_METADATA_CHANNEL,
                self.litellm_metadata
                    .as_ref()
                    .and_then(MetadataValue::value),
            ),
            (
                METADATA_CHANNEL,
                self.metadata.as_ref().and_then(MetadataValue::value),
            ),
        ]
    }
}

// track_deployment_metrics.find_deployment_metadata picks the first channel whose value is a bucket with a model_info key
#[must_use]
pub fn deployment_channel(channels: &CallbackChannels) -> Option<(&'static str, &MetadataBucket)> {
    for (name, bucket) in channels.named_buckets() {
        if let Some(bucket) = bucket.filter(|bucket| bucket.fields.contains_key(MODEL_INFO_KEY)) {
            return Some((name, bucket));
        }
    }
    None
}

// get_counted_usage_tokens: the first bucket with the key decides, mistyped value or not
#[must_use]
pub fn get_counted_usage_tokens(channels: &CallbackChannels) -> Option<i64> {
    for (_name, bucket) in channels.named_buckets() {
        if let Some(bucket) = bucket.filter(|bucket| bucket.fields.contains_key(COUNTED_TOKENS_KEY))
        {
            return bucket.counted_tokens();
        }
    }
    None
}
