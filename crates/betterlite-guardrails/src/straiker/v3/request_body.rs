use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

use crate::straiker::v3::credential_redaction::without_credentials;
use crate::straiker::v3::prompt_messages::prompt_as_messages;
use crate::straiker::v3::prompt_text::PromptTokenDecoder;
use crate::straiker::v3::route_detection::route_is;
use crate::straiker::v3::user_identity::real_identity;
use crate::straiker::v3::{IDENTITY_METADATA_KEYS, PROVIDER_BODY_KEYS, REDACTED_KEYS};
use crate::straiker_webhook::metadata::merged_metadata;

// The provider body LiteLLM received, stripped of everything the proxy added: logging objects, the resolved key, the inbound headers. Identity survives as the metadata subset Straiker's LiteLLM adapter reads. A call with no provider body, such as /guardrails/apply_guardrail with only `text`, relays `request_texts` as user turns.
#[must_use]
pub fn provider_body(
    request_data: &Object,
    request_texts: &[String],
    routes: &dyn RouteCallTypes<CallType>,
    decoder: &dyn PromptTokenDecoder,
) -> Object {
    let completion_turns = if route_is(request_data, CallType::TextCompletion, routes)
        && !request_data.contains_key("messages")
    {
        request_data
            .get("prompt")
            .and_then(|prompt| prompt_as_messages(prompt, decoder))
    } else {
        None
    };
    let has_conversation = request_data.get("messages").is_some_and(Json::is_truthy)
        || request_data.contains_key("prompt")
        || request_data.contains_key("input");
    let texts: Vec<Json> = request_texts
        .iter()
        .filter(|text| !text.is_empty())
        .map(|text| Json::Text(text.clone()))
        .collect();
    let text_turns = if completion_turns.is_none() && !has_conversation && !texts.is_empty() {
        prompt_as_messages(&Json::List(texts), decoder)
    } else {
        None
    };
    let mut body = Object::new();
    for (key, value) in request_data {
        let replaced = (key == "prompt" && completion_turns.is_some())
            || (key == "messages" && text_turns.is_some());
        if replaced || !PROVIDER_BODY_KEYS.contains(&key.as_str()) {
            continue;
        }
        let relayed = if REDACTED_KEYS.contains(&key.as_str()) {
            without_credentials(value)
        } else {
            value.clone()
        };
        body.insert(key.clone(), relayed);
    }
    if let Some(turns) = completion_turns.or(text_turns) {
        let messages = turns.into_iter().map(Json::Object).collect();
        body.insert("messages".to_owned(), Json::List(messages));
    }
    let identity = identity_metadata(request_data);
    if !identity.is_empty() {
        body.insert("metadata".to_owned(), Json::Object(identity));
    }
    body
}

// The proxy-resolved identity fields, and only those, for the relayed body.
fn identity_metadata(request_data: &Object) -> Object {
    let merged = merged_metadata(request_data);
    IDENTITY_METADATA_KEYS
        .iter()
        .filter_map(|&key| {
            let identity = real_identity(merged.get(key).and_then(Json::as_str))?;
            Some((key.to_owned(), Json::Text(identity.to_owned())))
        })
        .collect()
}

mod tests;
