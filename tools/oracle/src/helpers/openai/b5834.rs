use std::collections::BTreeMap;

use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Object as WireObject;
use betterlite_openai::perplexity_decisions::PerplexityDecisionsConfig;
use betterlite_openai::perplexity_embedding;
use betterlite_openai::perplexity_responses;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod replies;
mod search;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Embedding {
        urls: Vec<Option<String>>,
        headers: Vec<HeaderCall>,
        requests: Vec<EmbeddingCall>,
        replies: Vec<ReplyCall>,
        errors: Vec<ErrorCall>,
    },
    Responses {
        headers: Vec<HeaderCall>,
        urls: Vec<BaseCall>,
        requests: Vec<ResponsesCall>,
        replies: Vec<ReplyCall>,
    },
    Search {
        headers: Vec<SearchKeyCall>,
        urls: Vec<BaseCall>,
        requests: Vec<SearchCall>,
        replies: Vec<ReplyCall>,
    },
    Decisions {
        bases: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCall {
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaseCall {
    api_base: Option<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingCall {
    model: String,
    input: EmbeddingInput,
    non_default_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    body: String,
    status: u16,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    message: String,
    status: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesCall {
    model: String,
    input: ResponsesInput,
    params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchKeyCall {
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchCall {
    query: SearchQuery,
    optional_params: WireObject,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Embedding {
                urls,
                headers,
                requests,
                replies,
                errors,
            } => Ok(json!({
                "supported": perplexity_embedding::SUPPORTED_OPENAI_PARAMS,
                "urls": urls
                    .iter()
                    .map(|base| perplexity_embedding::get_complete_url(base.as_deref()))
                    .collect::<Vec<_>>(),
                "headers": headers.iter().map(embedding_headers).collect::<Vec<_>>(),
                "requests": requests.iter().map(embedding_request).collect::<Vec<_>>(),
                "replies": replies.iter().map(replies::embedding_reply).collect::<Vec<_>>(),
                "errors": errors.iter().map(error_view).collect::<Vec<_>>(),
            })),
            Input::Responses {
                headers,
                urls,
                requests,
                replies,
            } => {
                let url_values: Vec<String> = urls
                    .iter()
                    .map(|call| {
                        perplexity_responses::get_complete_url(call.api_base.as_deref(), &call.env)
                    })
                    .collect();
                Ok(json!({
                    "supported": perplexity_responses::SUPPORTED_OPENAI_PARAMS,
                    "websocket": perplexity_responses::SUPPORTS_NATIVE_WEBSOCKET,
                    "headers": headers.iter().map(responses_headers).collect::<Vec<_>>(),
                    "urls": url_values,
                    "requests": requests.iter().map(responses_request).collect::<Vec<_>>(),
                    "replies": replies::responses_replies(replies)?,
                }))
            }
            Input::Search {
                headers,
                urls,
                requests,
                replies,
            } => search::values(headers, urls, requests, replies),
            Input::Decisions { bases } => Ok(decisions(bases)),
        }
    }
}

// LiteLLM's helper clears the environment and sets only the case's variables
fn embedding_headers(call: &HeaderCall) -> Value {
    let key = call.api_key.clone().map(SecretString::from);
    let headers = perplexity_embedding::validate_environment(key.as_ref(), &call.env);
    json!({"ok": true, "headers": headers})
}

fn embedding_request(call: &EmbeddingCall) -> Value {
    let mut optional = Object::new();
    perplexity_embedding::map_openai_params(&call.non_default_params, &mut optional);
    perplexity_embedding::transform_embedding_request(&call.model, &call.input, &optional)
        .map_or_else(
            |_| json!({"raised": null}),
            |body| {
                let pairs: Vec<Value> = body
                    .iter()
                    .map(|(name, value)| json!([name, value]))
                    .collect();
                json!({"value": pairs})
            },
        )
}

fn error_view(call: &ErrorCall) -> Value {
    let error = perplexity_embedding::get_error_class(&call.message, call.status, &BTreeMap::new());
    json!({
        "status": error.status_code,
        "message": error.message,
        "method": error.request.method,
        "url": error.request.url,
        "response_status": error.response.status_code,
    })
}

fn responses_headers(call: &HeaderCall) -> Value {
    let key = call.api_key.clone().map(SecretString::from);
    let headers =
        perplexity_responses::validate_environment(call.headers.clone(), key.as_ref(), &call.env);
    json!({"ok": true, "headers": headers})
}

// a request LiteLLM rejects raises a ValueError, which the shared guard reports without a status
fn responses_request(call: &ResponsesCall) -> Value {
    perplexity_responses::transform_request(&call.model, &call.input, &call.params, None)
        .map_or_else(
            |_| json!({"raised": null}),
            |body| json!({"ok": true, "value": body}),
        )
}

fn decisions(bases: &[String]) -> Value {
    let config = PerplexityDecisionsConfig;
    json!({
        "path": PerplexityDecisionsConfig::PATH,
        "api_key_env": PerplexityDecisionsConfig::API_KEY_ENV,
        "api_base_env": PerplexityDecisionsConfig::API_BASE_ENV,
        "default_api_base": config.get_default_api_base(),
        "urls": bases
            .iter()
            .map(|base| config.get_complete_url(base, "pplx"))
            .collect::<Vec<_>>(),
    })
}
