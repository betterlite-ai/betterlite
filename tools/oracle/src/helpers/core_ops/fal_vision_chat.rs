use std::collections::BTreeMap;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_core::api::fal::vision_chat::headers::{complete_url, validate_environment};
use betterlite_core::api::fal::vision_chat::optional_params::{Params, map_openai_params};
use betterlite_core::api::fal::vision_chat::reply::{FalChatCompletion, transform_response};
use betterlite_core::api::fal::vision_chat::request::transform_request;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat::response::ChatResponse;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

type Environment = BTreeMap<String, String>;

const STAMPED_AT: u64 = 1_700_000_000;
const VALID_MESSAGES: &str = r#"[{"role": "user", "content": [{"type": "text", "text": "t"}, {"type": "image_url", "image_url": {"url": "u"}}]}]"#;
const STARTING_RESPONSE: &str = r#"{"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "old-model", "choices": [{"index": 0, "message": {"role": "assistant", "content": null}, "finish_reason": "stop"}]}"#;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    OneUserRequired {
        cases: Vec<RequestCase>,
    },
    JoinedText {
        cases: Vec<RequestCase>,
    },
    EmptyImageUrl {
        cases: Vec<RequestCase>,
    },
    ReasoningOverride {
        model: String,
        cases: Vec<MapCase>,
    },
    HeaderOverlay {
        environment: Environment,
        cases: Vec<HeaderCase>,
    },
    MalformedReply {
        model: String,
        replies: Vec<ReplyCase>,
    },
    ReasoningUsage {
        model: String,
        replies: Vec<ReplyCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCase {
    messages: Vec<ChatMessage>,
    optional_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCase {
    non_default: Params,
    optional: Params,
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCase {
    headers: ImageHeaders,
    api_key: Option<String>,
    api_base: Option<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    status: u16,
    headers: ImageHeaders,
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::OneUserRequired { cases }
            | Input::JoinedText { cases }
            | Input::EmptyImageUrl { cases } => {
                let outcomes: Vec<Value> = cases
                    .iter()
                    .map(|case| request_outcome(&case.messages, &case.optional_params))
                    .collect();
                Ok(json!({"cases": outcomes}))
            }
            Input::ReasoningOverride { model, cases } => {
                let messages: Vec<ChatMessage> = decode(VALID_MESSAGES)?;
                let outcomes: Vec<Value> = cases
                    .iter()
                    .map(|case| mapping_outcome(&messages, model, case))
                    .collect();
                Ok(json!({"cases": outcomes}))
            }
            Input::HeaderOverlay { environment, cases } => {
                let (api_key, api_base) = Sections::fal_image_environment(environment);
                let settings = FalImageSettings { api_key, api_base };
                let outcomes: Vec<Value> = cases
                    .iter()
                    .map(|case| header_outcome(&settings, case))
                    .collect();
                Ok(json!({"cases": outcomes}))
            }
            Input::MalformedReply { model, replies } | Input::ReasoningUsage { model, replies } => {
                let mut outcomes = Vec::new();
                for reply in replies {
                    let response: ChatResponse = decode(STARTING_RESPONSE)?;
                    outcomes.push(reply_outcome(model, reply, response));
                }
                Ok(json!({"cases": outcomes}))
            }
        }
    }
}

fn request_outcome(messages: &[ChatMessage], optional_params: &Params) -> Value {
    match transform_request(messages, optional_params) {
        Ok(body) => json!({"ok": true, "value": body}),
        Err(error) => json!({"ok": false, "status": error.status()}),
    }
}

fn mapping_outcome(messages: &[ChatMessage], model: &str, case: &MapCase) -> Value {
    match map_openai_params(
        &case.non_default,
        case.optional.clone(),
        model,
        case.drop_params,
    ) {
        Ok(mapped) => json!({
            "mapped": {"params": mapped},
            "request": request_outcome(messages, &mapped),
        }),
        Err(error) => json!({"mapped": {"error": {"status": error.status()}}}),
    }
}

fn header_outcome(settings: &FalImageSettings, case: &HeaderCase) -> Value {
    let url = complete_url(settings, case.api_base.as_deref(), &case.model);
    let explicit = case.api_key.as_deref().map(SecretString::from);
    match validate_environment(settings, &case.headers, explicit.as_ref()) {
        Ok(headers) => json!({"url": url, "headers": headers}),
        Err(error) => json!({"url": url, "error": {"status": error.status()}}),
    }
}

fn reply_outcome(model: &str, case: &ReplyCase, response: ChatResponse) -> Value {
    match transform_response(
        model,
        &case.headers,
        case.body.as_bytes(),
        response,
        STAMPED_AT,
    ) {
        Ok(completion) => json!({"ok": true, "value": completion_projection(completion)}),
        Err(error) => json!({
            "ok": false,
            "status": error.status(),
            "headers": error.headers(),
            "message": error.to_string(),
        }),
    }
}

fn completion_projection(completion: FalChatCompletion) -> Value {
    let response = completion.response;
    let choice = response.choices.first();
    json!({
        "id": response.id,
        "object": response.object,
        "created_stamped": response.created == STAMPED_AT,
        "model": response.model,
        "role": choice.map(|first| first.message.role.as_str()),
        "content": choice.and_then(|first| first.message.content.as_deref()),
        "reasoning_content": completion.reasoning_content,
        "finish_reason": choice.and_then(|first| first.finish_reason.as_deref()),
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn decode<T: DeserializeOwned>(text: &str) -> Result<T, Box<HelperError>> {
    serde_json::from_str(text).map_err(|error| {
        Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
