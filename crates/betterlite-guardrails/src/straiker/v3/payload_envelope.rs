use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

use crate::straiker::v3::RESPONSE_PHASE;
use crate::straiker::v3::answer_shape::{AnswerContext, AnthropicTranslator, Response, answer};
use crate::straiker::v3::error::V3Error;
use crate::straiker::v3::session_resolution::session_id;
use crate::straiker::v3::user_identity::user;
use crate::straiker_webhook::types::{StraikerWebhookContext, StraikerWebhookIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Request,
    Response,
}

pub struct PayloadInput<'input> {
    pub context: &'input StraikerWebhookContext,
    pub identity: &'input StraikerWebhookIdentity,
    pub texts: &'input [String],
    pub response: &'input Response,
    pub request_data: &'input Object,
    pub routes: &'input dyn RouteCallTypes<CallType>,
    pub translator: &'input dyn AnthropicTranslator,
}

// The /api/v3/detect body for one phase of a turn, the unified Kong plugin's contract. Request phase: the provider body itself. Response phase: the answer beside the request it answers, which is how Straiker classifies a tool call the model just made. Identity and session ride on both phases the way Kong sends them.
pub fn payload(
    input: &PayloadInput<'_>,
    phase: Phase,
    request_body: &Object,
) -> Result<Object, V3Error> {
    let mut body = match phase {
        Phase::Request => request_body.clone(),
        Phase::Response => response_phase(input, request_body)?,
    };
    let derived = session_id(
        input.context,
        input.identity,
        input.request_data,
        request_body,
    )?;
    if let Some(session) = derived {
        body.insert("session_id".to_owned(), Json::Text(session));
    }
    if let Some(principal) = user(input.identity) {
        let meta = Object::from([("user".to_owned(), Json::Text(principal))]);
        let processed = Object::from([("Meta".to_owned(), Json::Object(meta))]);
        let original = Object::from([("processed".to_owned(), Json::Object(processed))]);
        body.insert("original".to_owned(), Json::Object(original));
    }
    Ok(body)
}

fn response_phase(input: &PayloadInput<'_>, request_body: &Object) -> Result<Object, V3Error> {
    let model = input.context.model.as_deref();
    let mut body = Object::from([
        (
            "straiker_phase".to_owned(),
            Json::Text(RESPONSE_PHASE.to_owned()),
        ),
        (
            "model".to_owned(),
            model.map_or(Json::Null, |name| Json::Text(name.to_owned())),
        ),
        ("request".to_owned(), Json::Object(request_body.clone())),
    ]);
    let answer_context = AnswerContext {
        request_data: input.request_data,
        routes: input.routes,
        translator: input.translator,
        model,
    };
    if let Some(sse) = answer_json(input.response, input.texts, &answer_context)? {
        body.insert("sse".to_owned(), Json::Text(sse));
    }
    Ok(body)
}

// The model's answer as the raw response body Straiker parses on the response phase. The real response carries the tool calls a coding-agent turn is scored on, so it is preferred; a streamed answer reaches the hook as texts and becomes a minimal chat completion.
pub fn answer_json(
    response: &Response,
    texts: &[String],
    context: &AnswerContext<'_>,
) -> Result<Option<String>, V3Error> {
    if let Some(found) = answer(response, context).filter(|fields| !fields.is_empty()) {
        return serde_json::to_string(&found)
            .map(Some)
            .map_err(V3Error::Serialize);
    }
    let streamed: Vec<&str> = texts
        .iter()
        .map(String::as_str)
        .filter(|text| !text.is_empty())
        .collect();
    if streamed.is_empty() {
        return Ok(None);
    }
    let completion = serde_json::json!({
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": streamed.join("\n")},
        }],
    });
    Ok(Some(completion.to_string()))
}

mod tests;
