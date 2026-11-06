use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;

use betterlite_openai::chatgpt::chat::ChatGptError;
use betterlite_openai::chatgpt::headers::PlatformEnvironment;
use betterlite_openai::openai_base_config::{Headers, Object};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use self::fixture::{FixtureError, assemble};
use self::steps::step_value;
use crate::helpers::{Helper, HelperError, HelperResult};

mod fixture;
mod steps;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    platform: PlatformInput,
    #[serde(default)]
    record: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    now: Decimal,
    step: Step,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformInput {
    system: String,
    release: String,
    machine: String,
}

impl PlatformEnvironment for PlatformInput {
    fn os_type(&self) -> String {
        self.system.clone()
    }

    fn os_release(&self) -> String {
        self.release.clone()
    }

    fn arch(&self) -> String {
        self.machine.clone()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    ProviderInfo(ProviderInfoInput),
    Headers(HeadersInput),
    Params(ParamsInput),
    Normalize(NormalizeInput),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderInfoInput {
    model: String,
    provider: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadersInput {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: Headers,
    #[serde(default)]
    litellm_params: Object,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParamsInput {
    model: String,
    #[serde(default)]
    chat_models: BTreeSet<String>,
    #[serde(default)]
    text_models: BTreeSet<String>,
    requests: Vec<Object>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizeInput {
    chunks: Vec<Value>,
}

fn run_step(input: &Input) -> Result<Value, FixtureError> {
    let (api_key, headers) = match &input.step {
        Step::Headers(spec) => (
            spec.api_key.clone().map(SecretString::from),
            spec.headers.clone(),
        ),
        Step::ProviderInfo(_) | Step::Params(_) | Step::Normalize(_) => (None, Headers::new()),
    };
    let mut config = assemble(input, api_key, headers)?;
    let mut output = step_value(input, &mut config)?;
    let calls = config
        .authenticator
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    output.insert(String::from("authentication_calls"), json!(calls));
    Ok(Value::Object(output))
}

const fn failure_status(error: &FixtureError) -> Option<u16> {
    match error {
        FixtureError::Chat(ChatGptError::Authentication { .. }) => Some(401),
        FixtureError::Chat(ChatGptError::Login { source, .. }) => Some(source.status()),
        FixtureError::Config(_)
        | FixtureError::Store(_)
        | FixtureError::Json(_)
        | FixtureError::Runtime(_) => None,
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run_step(input).map_err(|error| {
            let status = failure_status(&error);
            Box::new(HelperError {
                class: String::from("ChatGptChatFixture"),
                status,
                error_type: Some(None),
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
