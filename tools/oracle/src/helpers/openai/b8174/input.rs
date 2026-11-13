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
        #[serde(default)]
        voice: Option<Voice>,
        #[serde(default)]
        params: Object,
    },
    SpeechBody {
        model: String,
        input: String,
        #[serde(default)]
        voice: Option<Voice>,
        #[serde(default)]
        params: Object,
    },
    SpeechUrl {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        env_base: Option<String>,
    },
    SpeechHeaders {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        global_key: Option<String>,
        #[serde(default)]
        env_key: Option<String>,
    },
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
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        env_base: Option<String>,
    },
    ImageHeaders {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        env_key: Option<String>,
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
#[serde(untagged)]
pub(crate) enum Voice {
    Text(String),
    Fields(Object),
}
