use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JudgeCall {
    pub model: String,
    pub messages: Vec<Json>,
    pub num_retries: u32,
    // LiteLLM passes no fallbacks argument on the SDK path, so None omits it instead of sending an empty list
    pub fallbacks: Option<Vec<Json>>,
    pub drop_params: bool,
    pub params: IndexMap<String, Json>,
}
