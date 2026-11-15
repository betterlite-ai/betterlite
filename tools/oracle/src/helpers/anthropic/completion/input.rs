use std::collections::BTreeMap;

use betterlite_anthropic::completion::prompt::CustomPromptEntry;
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;
use serde::Deserialize;

// one call carries one function and its cases, as the Python helper reads them
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Select {
        models: Vec<String>,
    },
    Prompt {
        cases: Vec<PromptCase>,
    },
    Auth {
        cases: Vec<AuthCase>,
    },
    Options {
        constructor: Constructor,
        cases: Vec<OptionCase>,
    },
    Reply {
        cases: Vec<ReplyCase>,
    },
    Chunk {
        chunks: Vec<IndexMap<String, Json>>,
    },
    Error {
        cases: Vec<ErrorCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptCase {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) custom_prompt: Option<CustomPromptEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthCase {
    pub(crate) api_key: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Constructor {
    pub(crate) max_tokens_to_sample: Option<i64>,
    pub(crate) stop_sequences: Option<Vec<String>>,
    pub(crate) top_k: Option<i64>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OptionCase {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) non_default: IndexMap<String, Json>,
    pub(crate) existing: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) initial_content: String,
}

// the supplied headers are read and dropped, as get_error_class never uses them
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCase {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    #[serde(rename = "headers")]
    pub(crate) _supplied_headers: BTreeMap<String, String>,
}
