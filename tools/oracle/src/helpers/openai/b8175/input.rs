use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    ChatSupportedParams,
    ChatDefaults {
        defaults: Object,
    },
    ChatMap {
        params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    EmbeddingSupportedParams,
    EmbeddingMap {
        params: Object,
        #[serde(default)]
        optional_params: Object,
        #[serde(default)]
        drop_params: bool,
    },
    EmbeddingUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    EmbeddingBody {
        model: String,
        input: EmbeddingInput,
        #[serde(default)]
        optional_params: Object,
    },
    EmbeddingHeaders {
        #[serde(default)]
        api_key: Option<String>,
    },
    EmbeddingReply {
        model: String,
        body: String,
    },
    BaseUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    Headers {
        api_key: String,
        #[serde(default)]
        extra_headers: Headers,
    },
    ErrorClass {
        message: String,
        status: u16,
        #[serde(default)]
        headers: Headers,
    },
}
