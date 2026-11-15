use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_config::sections::xai_identity::Settings;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::xai_responses::OauthAccess;
use betterlite_openai::xai_tokens::error::XaiOauthError;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

mod steps;
mod tests;
mod values;

const DEFAULT_OAUTH_BASE: &str = "https://api.x.ai/v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    env: BTreeMap<String, String>,
    xai_key: Option<String>,
    generic_key: Option<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Tools {
        params: Object,
    },
    Url {
        api_base: Option<String>,
        litellm_api_base: Option<String>,
        api_key: Option<String>,
        #[serde(default)]
        use_xai_oauth: bool,
    },
    Auth {
        api_key: Option<String>,
        #[serde(default)]
        use_xai_oauth: bool,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Reply {
        status: u16,
        body: String,
        now: i64,
    },
    Event {
        event: Value,
    },
    Capability,
}

// the case env points the token store at a path that never exists, so this login always fails
struct OauthFixture;

impl OauthAccess for OauthFixture {
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        ready(Err(XaiOauthError::LoginRequired))
    }
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn identity_settings(
    xai_key: Option<&str>,
    generic_key: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Settings {
    Settings {
        xai_key: xai_key.map(SecretString::from),
        generic_api_key: generic_key.map(SecretString::from),
        environment_key: env
            .get("XAI_API_KEY")
            .filter(|value| !value.is_empty())
            .cloned()
            .map(SecretString::from),
        api_base: env
            .get("XAI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
    }
}

fn oauth_base(env: &BTreeMap<String, String>) -> String {
    env.get("XAI_OAUTH_API_BASE")
        .filter(|value| !value.is_empty())
        .cloned()
        .or_else(|| {
            env.get("XAI_API_BASE")
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| String::from(DEFAULT_OAUTH_BASE))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = identity_settings(
            input.xai_key.as_deref(),
            input.generic_key.as_deref(),
            &input.env,
        );
        // LiteLLM raises per call, so the case keeps every step's answer beside a raised status
        let answers = input
            .steps
            .iter()
            .map(|one| match steps::run_step(one, &settings, &input.env) {
                Ok(value) => value,
                Err(error) => json!({"raised": error.status}),
            })
            .collect::<Vec<Value>>();
        Ok(Value::Array(answers))
    }
}
