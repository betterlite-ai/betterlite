use std::collections::{BTreeMap, BTreeSet};
use std::iter::once;

use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::OpenAiSettings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_provider::azure_dispatch::call::{
    ChatCall, Composition, Credentials, EmbeddingCall, Globals, Request, TextCall,
};
use betterlite_provider::azure_dispatch::fixtures::execution::dispatch;
use betterlite_provider::azure_dispatch::fixtures::types::{Answer, AnswerReply, AzureResult};
use betterlite_provider::prepared::PreparedHttp;
use http::HeaderMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::case::{Case, Operation, failure};

const KEPT_HEADERS: [&str; 3] = ["authorization", "content-type", "api-key"];

// The answer port: the case's body answers every request, and no connection is opened.
struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Answer for Reply {
    fn answer(&self, _request: &PreparedHttp) -> AnswerReply {
        AnswerReply {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }
    }
}

// A configuration section is the same JSON in both languages: main.py's class defaults merge only absent keys.
fn composition(case: &Case) -> Result<Composition, Box<HelperError>> {
    let chat_defaults = match case.chat_defaults.as_ref() {
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(|error| failure("ValueError", error))?
        }
        None => AzureChatSettings::default(),
    };
    let o_series_defaults = match case.o_series_defaults.as_ref() {
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(|error| failure("ValueError", error))?
        }
        None => OpenAiSettings::default(),
    };
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
        chat_defaults,
        o_series_defaults,
        openai_system_messages_first: false,
    })
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, Box<HelperError>> {
    serde_json::from_value(value.clone()).map_err(|error| failure("ValueError", error))
}

fn messages(case: &Case) -> Result<Vec<Message>, Box<HelperError>> {
    serde_json::from_value(Value::Array(case.messages.clone()))
        .map_err(|error| failure("ValueError", error))
}

fn optional(case: &Case) -> Result<Object, Box<HelperError>> {
    match case.optional_params.as_ref() {
        Some(value) => decode(value),
        None => Ok(Object::new()),
    }
}

fn call_credentials(case: &Case) -> Result<Credentials, Box<HelperError>> {
    Ok(Credentials {
        api_base: case.api_base.clone(),
        api_version: case.api_version.clone(),
        api_key: case.api_key.clone().map(SecretString::from),
        headers: case.headers.clone(),
        extra_headers: case.extra_headers.clone(),
        max_retries: match case.max_retries.as_ref() {
            Some(value) => Some(decode(value)?),
            None => None,
        },
        extra_body: match case.extra_body.as_ref() {
            Some(value) => Some(decode(value)?),
            None => None,
        },
        azure_ad_token: case.azure_ad_token.clone().map(SecretString::from),
        base_model: None,
        timeout: None,
    })
}

fn request(case: &Case) -> Result<Request, Box<HelperError>> {
    let credentials = call_credentials(case)?;
    match case.operation {
        Operation::Chat => Ok(Request::Chat(Box::new(ChatCall {
            model: case.model.clone(),
            messages: messages(case)?,
            optional: optional(case)?,
            credentials,
            stream: false,
            supplied: None,
        }))),
        Operation::Embedding => {
            let Some(value) = case.embedding.as_ref() else {
                return Err(failure("ValueError", "an embedding case names its body"));
            };
            let request: EmbeddingRequest = decode(value)?;
            Ok(Request::Embedding(Box::new(EmbeddingCall {
                model: case.model.clone(),
                request,
                credentials,
            })))
        }
        Operation::Text => Ok(Request::Text(Box::new(TextCall {
            model: case.model.clone(),
            messages: messages(case)?,
            optional: optional(case)?,
            credentials,
            acompletion: case.acompletion,
        }))),
    }
}

fn kept_headers(case: &Case) -> BTreeSet<String> {
    let mut keep: BTreeSet<String> = KEPT_HEADERS.iter().map(|name| (*name).to_owned()).collect();
    for step in once(case).chain(case.second.as_deref()) {
        for map in [&step.headers, &step.extra_headers] {
            if let Some(map) = map {
                keep.extend(map.keys().map(|name| name.to_ascii_lowercase()));
            }
        }
    }
    keep
}

fn requests_view(
    result: &AzureResult,
    keep: &BTreeSet<String>,
) -> Result<Vec<Value>, Box<HelperError>> {
    result
        .requests
        .iter()
        .map(|record| {
            let headers: BTreeMap<String, String> = record
                .headers
                .iter()
                .filter(|(name, _value)| keep.contains(name.to_ascii_lowercase().as_str()))
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .collect();
            let body =
                serde_json::to_value(&record.body).map_err(|error| failure("ValueError", error))?;
            Ok(serde_json::json!({
                "method": record.method.clone(),
                "url": record.url.clone(),
                "headers": headers,
                "body": body,
            }))
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub(crate) struct Observation {
    pub(crate) requests: Vec<Value>,
    pub(crate) replies: Vec<Value>,
    pub(crate) error: Value,
}

// The two-step replay: each call composes against the same configuration and the module globals.
pub(crate) fn run(case: &Case) -> Result<Observation, Box<HelperError>> {
    let composition = composition(case)?;
    let body = serde_json::to_vec(&case.reply).map_err(|error| failure("ValueError", error))?;
    let answer = Reply {
        status: case.status,
        body,
    };
    let keep = kept_headers(case);
    let mut requests = Vec::new();
    let mut replies = Vec::new();
    let mut error = Value::Null;
    for step in once(case).chain(case.second.as_deref()) {
        let result = dispatch(&composition, request(step)?, &answer);
        requests.extend(requests_view(&result, &keep)?);
        let reply =
            serde_json::to_value(&result.reply).map_err(|error| failure("ValueError", error))?;
        if result.error == Json::Null {
            replies.push(reply);
        } else {
            error = serde_json::to_value(&result.error)
                .map_err(|error| failure("ValueError", error))?;
            break;
        }
    }
    Ok(Observation {
        requests,
        replies,
        error,
    })
}
