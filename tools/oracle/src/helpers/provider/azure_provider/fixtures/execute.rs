use std::collections::{BTreeMap, BTreeSet};
use std::iter::once;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Json;
use betterlite_provider::azure_dispatch::call::{Composition, Globals};
use betterlite_provider::azure_dispatch::fixtures::execution::dispatch;
use betterlite_provider::azure_dispatch::fixtures::types::{Answer, AnswerReply, AzureResult};
use betterlite_provider::prepared::PreparedHttp;
use http::HeaderMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::case::{Case, failure};
use super::request::{decode, request};

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

// A default arrives either as the value the case names or as the class defaults, which merge only absent keys.
fn defaults<T>(value: Option<&Value>) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned + Default,
{
    let Some(named) = value else {
        return Ok(T::default());
    };
    decode(named)
}

fn kept_headers(case: &Case) -> BTreeSet<String> {
    let mut keep: BTreeSet<String> = KEPT_HEADERS.iter().map(|name| (*name).to_owned()).collect();
    for step in once(case).chain(case.second.as_deref()) {
        for named in [&step.headers, &step.extra_headers] {
            if let Some(map) = named {
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
    // The configuration section is the same JSON in both languages, so both sides deserialize it the same way.
    let composition = Composition {
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
    };
    let body = serde_json::to_vec(&case.reply).map_err(|cause| failure("ValueError", cause))?;
    let answer = Reply {
        status: case.status,
        body,
    };
    replay(&composition, case, &answer, &kept_headers(case))
}

fn replay(
    composition: &Composition,
    case: &Case,
    answer: &Reply,
    keep: &BTreeSet<String>,
) -> Result<Observation, Box<HelperError>> {
    let mut requests = Vec::new();
    let mut replies = Vec::new();
    let mut error = Value::Null;
    for step in once(case).chain(case.second.as_deref()) {
        let result = dispatch(composition, request(step)?, answer);
        requests.extend(requests_view(&result, keep)?);
        let reply =
            serde_json::to_value(&result.reply).map_err(|cause| failure("ValueError", cause))?;
        if result.error == Json::Null {
            replies.push(reply);
        } else {
            error = serde_json::to_value(&result.error)
                .map_err(|cause| failure("ValueError", cause))?;
            break;
        }
    }
    Ok(Observation {
        requests,
        replies,
        error,
    })
}
