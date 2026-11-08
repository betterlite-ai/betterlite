use serde::Deserialize;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Constructor(ConstructorInput),
    KeyScope(KeyScopeInput),
    IndexFallback(IndexFallbackInput),
    Lookup(LookupInput),
    Store(StoreInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructorInput {
    #[serde(default)]
    pub(crate) threshold: Option<f64>,
    #[serde(default)]
    pub(crate) host: Option<String>,
    #[serde(default)]
    pub(crate) port: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    #[serde(default)]
    pub(crate) redis_url: Option<String>,
    #[serde(default)]
    pub(crate) index_name: Option<String>,
    #[serde(default)]
    pub(crate) embedding_model: Option<String>,
    #[serde(default)]
    pub(crate) embedding_max_input_tokens: Option<i64>,
    pub(crate) embedding_timeout: f64,
    #[serde(default)]
    pub(crate) env: ConstructorEnv,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructorEnv {
    #[serde(default)]
    pub(crate) redis_host: Option<String>,
    #[serde(default)]
    pub(crate) redis_port: Option<String>,
    #[serde(default)]
    pub(crate) redis_password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyScopeInput {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) litellm_cache_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndexFallbackInput {
    pub(crate) index_name: String,
    pub(crate) dimension: usize,
    #[serde(default)]
    pub(crate) mismatch_attempts: Vec<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LookupInput {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) messages: Option<Vec<Message>>,
    #[serde(default, rename = "input")]
    pub(crate) responses_input: Option<Json>,
    #[serde(default)]
    pub(crate) metadata: Option<Json>,
    pub(crate) vector: Vec<f32>,
    #[serde(default)]
    pub(crate) hits: Vec<HitInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HitInput {
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) response: Option<String>,
    #[serde(default)]
    pub(crate) litellm_cache_key: Option<String>,
    #[serde(default)]
    pub(crate) vector_distance: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreInput {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) messages: Option<Vec<Message>>,
    #[serde(default, rename = "input")]
    pub(crate) responses_input: Option<Json>,
    #[serde(default)]
    pub(crate) metadata: Option<Json>,
    #[serde(default)]
    pub(crate) ttl: Option<i64>,
    pub(crate) vector: Vec<f32>,
    pub(crate) value: Json,
}
