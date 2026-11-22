use std::collections::{BTreeMap, BTreeSet};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Json;
use betterlite_provider::azure_dispatch::AzureResult;
use betterlite_provider::azure_dispatch::call::{ChatCall, Composition, EmbeddingCall, TextCall};
use betterlite_provider::azure_dispatch::dispatch::{self, Answer, AnswerReply};
use betterlite_provider::prepared::PreparedHttp;
use http::HeaderMap;
use serde_json::Value;

use super::{
    Case, KEPT_HEADERS, Operation, composition, credentials, embedding_request, messages, optional,
    value_failure,
};

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

fn run_step(
    case: &Case,
    composition: &Composition,
    answer: &Reply,
) -> Result<AzureResult, Box<HelperError>> {
    let credentials = credentials(case)?;
    match case.operation {
        Operation::Chat => Ok(dispatch::dispatch_chat(
            composition,
            ChatCall {
                model: case.model.clone(),
                messages: messages(case)?,
                optional: optional(case)?,
                credentials,
                stream: false,
                supplied: None,
            },
            answer,
        )),
        Operation::Embedding => Ok(dispatch::dispatch_embedding(
            composition,
            EmbeddingCall {
                model: case.model.clone(),
                request: embedding_request(case)?,
                credentials,
            },
            answer,
        )),
        Operation::Text => Ok(dispatch::dispatch_text(
            composition,
            TextCall {
                model: case.model.clone(),
                messages: messages(case)?,
                optional: optional(case)?,
                credentials,
                acompletion: case.acompletion,
            },
            answer,
        )),
    }
}

fn steps(case: &Case) -> impl Iterator<Item = &Case> {
    std::iter::once(case).chain(case.second.as_deref())
}

fn kept_headers(case: &Case) -> BTreeSet<String> {
    let mut keep: BTreeSet<String> = KEPT_HEADERS.iter().map(|name| (*name).to_owned()).collect();
    for step in steps(case) {
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
            let body = serde_json::to_value(&record.body).map_err(value_failure)?;
            Ok(serde_json::json!({
                "method": record.method,
                "url": record.url,
                "headers": headers,
                "body": body,
            }))
        })
        .collect()
}

pub(super) fn dispatch_case(case: &Case) -> HelperResult {
    let composition = composition(case)?;
    let body = serde_json::to_vec(&case.reply).map_err(value_failure)?;
    let answer = Reply {
        status: case.status,
        body,
    };
    let keep = kept_headers(case);
    let mut requests = Vec::new();
    let mut replies = Vec::new();
    let mut error = Value::Null;
    for step in steps(case) {
        let result = run_step(step, &composition, &answer)?;
        requests.extend(requests_view(&result, &keep)?);
        if result.error == Json::Null {
            replies.push(serde_json::to_value(&result.reply).map_err(value_failure)?);
        } else {
            error = serde_json::to_value(&result.error).map_err(value_failure)?;
            break;
        }
    }
    Ok(serde_json::json!({
        "requests": requests,
        "replies": replies,
        "error": error,
    }))
}
