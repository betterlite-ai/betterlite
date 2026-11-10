use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) wrapper_keys: TransformCase,
    pub(crate) context_management_removed: TransformCase,
    pub(crate) missing_predictions: Json,
    pub(crate) embedded_429: Json,
    pub(crate) invalid_error_code: Json,
    pub(crate) coercible_error_code: Json,
    pub(crate) fake_stream_flag: FakeStreamCase,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCase {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) litellm_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FakeStreamCase {
    pub(crate) cases: Vec<FakeStreamQuery>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FakeStreamQuery {
    pub(crate) model: Option<String>,
    pub(crate) stream: Option<bool>,
    pub(crate) provider: Option<String>,
}
