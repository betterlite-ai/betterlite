use serde::Deserialize;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    #[default]
    Constructor,
    KeyScope,
    IndexFallback,
    Lookup,
    Store,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) threshold: Option<f64>,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) redis_url: Option<String>,
    pub(crate) index_name: Option<String>,
    pub(crate) embedding_model: Option<String>,
    pub(crate) embedding_max_input_tokens: Option<i64>,
    pub(crate) embedding_timeout: Option<f64>,
    pub(crate) redis_host: Option<String>,
    pub(crate) redis_port: Option<String>,
    pub(crate) redis_password: Option<String>,
    pub(crate) key: Option<String>,
    pub(crate) litellm_cache_key: Option<String>,
    pub(crate) dimension: Option<usize>,
    pub(crate) mismatch_attempts: Vec<usize>,
    pub(crate) messages: Option<Vec<Message>>,
    pub(crate) input: Option<Json>,
    pub(crate) metadata: Option<Json>,
    pub(crate) vector: Option<Vec<f32>>,
    pub(crate) hits: Vec<HitInput>,
    pub(crate) ttl: Option<i64>,
    pub(crate) value: Option<Json>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct HitInput {
    pub(crate) prompt: Option<String>,
    pub(crate) response: Option<String>,
    pub(crate) litellm_cache_key: Option<String>,
    pub(crate) vector_distance: Option<f64>,
}
