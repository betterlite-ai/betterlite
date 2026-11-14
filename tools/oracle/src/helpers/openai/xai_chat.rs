use std::fmt;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod logs;
mod oauth;
mod ops;
mod tests;

use ops::Run;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    settings: Settings,
    steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    api_base: Option<String>,
    oauth_api_base: Option<String>,
    xai_key: Option<SecretString>,
    environment_key: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Supported {
        model: String,
        supports_reasoning: Option<bool>,
    },
    Map {
        model: String,
        supports_reasoning: bool,
        params: Vec<(String, Json)>,
        optional_params: Object,
    },
    Request {
        model: String,
        messages: Vec<ChatMessage>,
        optional_params: Object,
    },
    Environment {
        model: String,
        headers: Headers,
        api_key: Option<SecretString>,
        use_xai_oauth: bool,
    },
    Url {
        api_base: Option<String>,
        api_key: Option<SecretString>,
        use_xai_oauth: bool,
    },
    Reply {
        body: String,
    },
    Chunk {
        chunk: Value,
    },
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

const CHAT_FAILURE: &str = "XaiChatError";
const STREAM_FAILURE: &str = "XaiStreamError";

fn failure<E>(class: &str, status: Option<u16>, error: E) -> Box<HelperError>
where
    E: fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| invalid(error.to_string()))?;
        let run = Run {
            settings: &input.settings,
            runtime: &runtime,
        };
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(run.step(step)?);
        }
        Ok(Value::Array(results))
    }
}
