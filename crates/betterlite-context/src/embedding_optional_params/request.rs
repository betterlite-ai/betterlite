use crate::non_default_params::Params;
use crate::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};

pub struct EmbeddingRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub passed_params: &'request Params,
    pub special_params: &'request Params,
    pub additional_drop_params: &'request [String],
    pub allowed_openai_params: &'request [String],
    pub drop_policy: DropPolicy,
    pub provider_config: Option<&'request dyn OptionalParamsMapper>,
    pub mapper: Option<&'request dyn OptionalParamsMapper>,
    pub supported: Option<&'request [String]>,
}
