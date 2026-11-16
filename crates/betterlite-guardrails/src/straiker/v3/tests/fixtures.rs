#![cfg(test)]

use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;
use serde::Serialize;

use crate::straiker::v3::answer_shape::AnthropicTranslator;
use crate::straiker::v3::prompt_text::PromptTokenDecoder;
use crate::straiker_webhook::types::{StraikerWebhookContext, StraikerWebhookIdentity};

pub(super) fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn object(source: &str) -> Object {
    match parsed(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

pub(super) fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

pub(super) fn context(session_id: Option<&str>) -> StraikerWebhookContext {
    StraikerWebhookContext {
        call_surface: "completion".to_owned(),
        mode: None,
        model: Some("gpt-4o".to_owned()),
        model_provider: None,
        destination: None,
        session_id: session_id.map(str::to_owned),
        litellm_call_id: None,
        litellm_trace_id: None,
        litellm_version: None,
    }
}

const NOBODY: StraikerWebhookIdentity = StraikerWebhookIdentity {
    litellm_key: None,
    litellm_team: None,
    litellm_user_id: None,
    litellm_user_email: None,
    litellm_org_id: None,
    end_user_id: None,
};

pub(super) fn identity(user_id: Option<&str>) -> StraikerWebhookIdentity {
    let mut identity = NOBODY;
    identity.litellm_user_id = user_id.map(str::to_owned);
    identity
}

pub(super) struct Routes;

impl RouteCallTypes<CallType> for Routes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<CallType>> {
        match route {
            "/v1/completions" => Some(vec![CallType::TextCompletion]),
            "/v1/messages" => Some(vec![CallType::AnthropicMessages]),
            "/v1/chat/completions" => Some(vec![CallType::Completion]),
            _ => None,
        }
    }
}

// Ids 1 to 99 decode to "t<id>" joined by spaces, and any other id has no token.
pub(super) struct Tokens;

impl PromptTokenDecoder for Tokens {
    fn decode(&self, ids: &[u32]) -> Option<String> {
        let words: Vec<String> = ids
            .iter()
            .map(|id| (1..100).contains(id).then(|| format!("t{id}")))
            .collect::<Option<_>>()?;
        Some(words.join(" "))
    }
}

pub(super) struct NoTokenizer;

impl PromptTokenDecoder for NoTokenizer {
    fn decode(&self, _ids: &[u32]) -> Option<String> {
        None
    }
}

pub(super) struct Translator;

impl AnthropicTranslator for Translator {
    fn translate(&self, response: &Object) -> Object {
        let id = response.get("id").cloned().unwrap_or(Json::Null);
        Object::from([
            ("id".to_owned(), id),
            ("type".to_owned(), Json::Text("message".to_owned())),
            ("model".to_owned(), Json::Text("translated".to_owned())),
        ])
    }
}
