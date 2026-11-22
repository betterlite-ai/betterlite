use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_provider::azure_dispatch::call::{Composition, Credentials, Globals};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;

mod run;
mod tests;

const DEFAULT_STATUS: u16 = 200;
const KEPT_HEADERS: [&str; 3] = ["authorization", "content-type", "api-key"];

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        run::dispatch_case(input)
    }
}

fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

fn value_failure(error: serde_json::Error) -> Box<HelperError> {
    failure("ValueError", error)
}

const fn default_status() -> u16 {
    DEFAULT_STATUS
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Chat,
    Embedding,
    Text,
}

#[derive(Debug, Default, Deserialize)]
struct CaseGlobals {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    azure_key: Option<String>,
    #[serde(default)]
    headers: Option<Headers>,
    #[serde(default)]
    azure_default_api_version: Option<String>,
}

// One Azure call, keyed the way main.py reads its own arguments; a second call proves per-call resolution.
#[derive(Debug, Deserialize)]
pub(crate) struct Case {
    #[serde(rename = "case")]
    operation: Operation,
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    azure_ad_token: Option<String>,
    #[serde(default)]
    headers: Option<Headers>,
    #[serde(default)]
    extra_headers: Option<Headers>,
    #[serde(default)]
    extra_body: Option<Value>,
    #[serde(default)]
    max_retries: Option<Value>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    globals: CaseGlobals,
    #[serde(default)]
    chat_defaults: Option<Value>,
    #[serde(default)]
    o_series_defaults: Option<Value>,
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default)]
    optional_params: Option<Value>,
    #[serde(default)]
    embedding: Option<Value>,
    #[serde(default)]
    acompletion: bool,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    reply: Value,
    #[serde(default)]
    second: Option<Box<Case>>,
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, Box<HelperError>> {
    serde_json::from_value(value.clone()).map_err(value_failure)
}

fn defaults<T: Default + serde::de::DeserializeOwned>(
    value: Option<&Value>,
) -> Result<T, Box<HelperError>> {
    value.map_or_else(|| Ok(T::default()), decode)
}

fn messages(case: &Case) -> Result<Vec<Message>, Box<HelperError>> {
    decode(&Value::Array(case.messages.clone()))
}

fn optional(case: &Case) -> Result<Object, Box<HelperError>> {
    match case.optional_params.as_ref() {
        None | Some(Value::Null) => Ok(Object::new()),
        Some(value) => decode(value),
    }
}

fn embedding_request(case: &Case) -> Result<EmbeddingRequest, Box<HelperError>> {
    match case.embedding.as_ref() {
        Some(value) => decode(value),
        None => Err(failure("ValueError", "an embedding case names its body")),
    }
}

// A configuration section is the same JSON in both languages: main.py's class defaults merge only absent keys.
fn composition(case: &Case) -> Result<Composition, Box<HelperError>> {
    Ok(Composition {
        environment: case.environment.clone(),
        globals: Globals {
            api_base: case.globals.api_base.clone(),
            api_version: case.globals.api_version.clone(),
            api_key: case.globals.api_key.clone().map(SecretString::from),
            azure_key: case.globals.azure_key.clone().map(SecretString::from),
            headers: case.globals.headers.clone(),
            azure_default_api_version: case.globals.azure_default_api_version.clone(),
        },
        chat_defaults: defaults(case.chat_defaults.as_ref())?,
        o_series_defaults: defaults(case.o_series_defaults.as_ref())?,
        openai_system_messages_first: false,
    })
}

fn credentials(case: &Case) -> Result<Credentials, Box<HelperError>> {
    let max_retries = case.max_retries.as_ref().map(decode).transpose()?;
    let extra_body = case.extra_body.as_ref().map(decode).transpose()?;
    Ok(Credentials {
        api_base: case.api_base.clone(),
        api_version: case.api_version.clone(),
        api_key: case.api_key.clone().map(SecretString::from),
        headers: case.headers.clone(),
        extra_headers: case.extra_headers.clone(),
        max_retries,
        extra_body,
        azure_ad_token: case.azure_ad_token.clone().map(SecretString::from),
        ..Credentials::default()
    })
}
