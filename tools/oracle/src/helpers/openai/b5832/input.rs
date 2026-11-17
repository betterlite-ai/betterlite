use betterlite_openai::fireworks_ai_common::SessionMetadata;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object as Params;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) catalog: Vec<CatalogModel>,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogModel {
    pub(crate) key: String,
    pub(crate) model_info: Box<RawValue>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct EnvCall {
    pub(crate) fireworks_api_key: Option<String>,
    pub(crate) fireworks_ai_api_key: Option<String>,
    pub(crate) fireworksai_api_key: Option<String>,
    pub(crate) fireworks_ai_token: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionCall {
    pub(crate) litellm_session_id: Option<String>,
    pub(crate) litellm_metadata: Option<SessionMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Supported,
    Capability,
    MapParams {
        #[serde(default)]
        non_default_params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    ExtraBody {
        model: String,
        #[serde(default)]
        optional_params: Object,
    },
    Completion {
        model: String,
        #[serde(default)]
        messages: Vec<Value>,
        #[serde(default)]
        optional_params: Object,
    },
    Headers {
        #[serde(default)]
        headers: Headers,
        api_key: Option<String>,
        #[serde(default)]
        env: EnvCall,
        #[serde(default)]
        session: SessionCall,
    },
    Url {
        api_base: Option<String>,
        #[serde(default)]
        env: EnvCall,
    },
    Transform {
        model: String,
        input: ResponsesInput,
        #[serde(default)]
        optional_params: Params,
    },
    Delete {
        path: String,
    },
}
