use super::args::{CompletionArgs, JsonObject};
use betterlite_context::non_default_params::Params;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_router::routing_timeouts::completion::TimeoutValue;
use secrecy::SecretString;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedOptionalParams {
    pub params: Params,
    pub dropped_fields: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct PreparedCall {
    pub model: String,
    pub custom_llm_provider: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Option<Vec<Json>>,
    pub kwargs: JsonObject,
    pub options: PreparedOptionalParams,
    pub non_default_params: JsonObject,
    pub litellm_params: CallParams,
    pub headers: AdditionalHeaders,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<TimeoutValue>,
    pub max_retries: Option<i64>,
    pub stream: bool,
    pub choice_count: Option<u64>,
    pub model_info: JsonObject,
    pub hidden_params: JsonObject,
    pub custom_prompt_dict: JsonObject,
    pub base_model: Option<String>,
}
#[derive(Clone, Debug)]
pub struct FallbackCall {
    pub fallbacks: Vec<Json>,
    pub args: CompletionArgs,
}
#[derive(Clone, Debug)]
pub struct ModelListCall {
    pub deployments: Vec<JsonObject>,
    pub args: CompletionArgs,
}
#[derive(Clone, Debug)]
pub struct MockCall {
    pub call: PreparedCall,
    pub acompletion: bool,
    pub mock_delay: Option<Json>,
    pub prompt_tokens: Option<i64>,
}
#[derive(Clone, Debug)]
pub enum PreparedCompletion {
    Dispatch(PreparedCall),
    Fallbacks(FallbackCall),
    ModelList(ModelListCall),
    Mock(MockCall),
    ResponsesBridge(PreparedCall),
    McpLoop(PreparedCall),
}
