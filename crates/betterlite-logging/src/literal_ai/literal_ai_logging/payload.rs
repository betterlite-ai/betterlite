use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

// The callback kwargs _prepare_log_data reads: the standard logging object and the model names it puts on the step.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackKwargs {
    #[serde(default)]
    pub standard_logging_object: Option<StandardLoggingPayload>,
    #[serde(default)]
    pub litellm_params: Option<LitellmParams>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LitellmParams {
    #[serde(default)]
    pub metadata: Option<IndexMap<String, Json>>,
}

// litellm.types.utils.StandardLoggingPayload, the fields _prepare_log_data reads.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StandardLoggingPayload {
    #[serde(default)]
    pub metadata: IndexMap<String, Json>,
    #[serde(default)]
    pub model_parameters: IndexMap<String, Json>,
    #[serde(default)]
    pub messages: Option<Json>,
    #[serde(default)]
    pub response: Option<Json>,
    #[serde(default)]
    pub error_str: Option<String>,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}
