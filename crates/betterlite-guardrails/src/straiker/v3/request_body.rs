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
    let completion_turns = completion_turns_from_prompt(request_data, routes, decoder);
    let text_turns = turns_from_request_texts(
        request_data,
        request_texts,
        completion_turns.is_some(),
        decoder,
    );
    let mut body = relayed_body(
        request_data,
        completion_turns.as_deref(),
        text_turns.is_some(),
    );
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

// A completions `prompt` is decoded only on the completions route; a call that also carries messages is relayed as sent.
fn completion_turns_from_prompt(
    request_data: &Object,
    routes: &dyn RouteCallTypes<CallType>,
    decoder: &dyn PromptTokenDecoder,
) -> Option<Vec<Object>> {
    let on_completions = route_is(request_data, CallType::TextCompletion, routes)
        && !request_data.contains_key("messages");
    if !on_completions {
        return None;
    }
    request_data
        .get("prompt")
        .and_then(|prompt| prompt_as_messages(prompt, decoder))
}

// The hook's request_texts stand in for a provider body a call never had, such as /guardrails/apply_guardrail with only `text`.
fn turns_from_request_texts(
    request_data: &Object,
    request_texts: &[String],
    has_completion_turns: bool,
    decoder: &dyn PromptTokenDecoder,
) -> Option<Vec<Object>> {
    let has_conversation = request_data.get("messages").is_some_and(Json::is_truthy)
        || request_data.contains_key("prompt")
        || request_data.contains_key("input");
    let texts: Vec<Json> = request_texts
        .iter()
        .filter(|text| !text.is_empty())
        .map(|text| Json::Text(text.clone()))
        .collect();
    if has_completion_turns || has_conversation || texts.is_empty() {
        return None;
    }
    prompt_as_messages(&Json::List(texts), decoder)
}

fn relayed_body(
    request_data: &Object,
    completion_turns: Option<&[Object]>,
    messages_are_replaced: bool,
) -> Object {
    let mut body = Object::new();
    for (key, value) in request_data {
        let replaced = (key == "prompt" && completion_turns.is_some())
            || (key == "messages" && messages_are_replaced);
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
