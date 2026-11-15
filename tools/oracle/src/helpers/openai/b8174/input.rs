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
    SpeechParams {
        voice: Option<Voice>,
        #[serde(default)]
        params: Object,
    },
    SpeechBody {
        model: String,
        input: String,
        voice: Option<Voice>,
        #[serde(default)]
        params: Object,
    },
    SpeechUrl {
        api_base: Option<String>,
        env_base: Option<String>,
    },
    SpeechHeaders(SpeechHeaderKeys),
    SpeechReply {
        status: u16,
        #[serde(default)]
        headers: Headers,
        body: String,
    },
    ImageParams {
        model: String,
        #[serde(default)]
        params: Object,
        #[serde(default)]
        drop_params: bool,
    },
    ImageUrl {
        api_base: Option<String>,
        env_base: Option<String>,
    },
    ImageBody {
        model: String,
        prompt: String,
        #[serde(default)]
        params: Object,
    },
    ImageReply {
        status: u16,
        body: String,
        #[serde(default)]
        created: i64,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpeechHeaderKeys {
    pub(crate) api_key: Option<String>,
    pub(crate) global_key: Option<String>,
    pub(crate) env_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Voice {
    Text(String),
    Fields(Object),
}
