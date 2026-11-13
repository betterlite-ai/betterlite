use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::unions::{
    OutputMessageContentValue, SdkOutput,
};
use indexmap::IndexMap;
use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct StopDetails {
    pub r#type: &'static str,
    pub category: Option<String>,
    pub explanation: Option<String>,
}

// LiteLLM reads these two from the typed Message or Delta, so a raw chat dict never reaches it.
pub struct ChatTurn<'turn> {
    pub refusal: &'turn Json,
    pub provider_specific_fields: &'turn Json,
}

pub enum OutputItem<'item> {
    Typed(&'item SdkOutput),
    Raw(&'item Json),
}

#[must_use]
pub const fn refusal_stop_details(explanation: Option<String>) -> StopDetails {
    StopDetails {
        r#type: "refusal",
        category: None,
        explanation,
    }
}

// An empty stop_details object is still a safeguard refusal, so presence decides, not truthiness.
#[must_use]
pub fn safeguard_refusal_stop_details(response: &Json) -> Option<&IndexMap<String, Json>> {
    let Json::Object(fields) = response else {
        return None;
    };
    if !matches!(fields.get("stop_reason"), Some(Json::Text(reason)) if reason == "refusal") {
        return None;
    }
    match fields.get("stop_details") {
        Some(Json::Object(details)) => Some(details),
        _ => None,
    }
}

#[must_use]
pub fn safeguard_refusal_error(model: &str, stop_details: &IndexMap<String, Json>) -> ErrorClass {
    let category = match stop_details.get("category") {
        Some(Json::Text(category)) => category.as_str(),
        _ => "None",
    };
    ErrorClass::ContentPolicyViolation(Box::new(ResponseDetails::from(ErrorContext {
        message: format!("Anthropic safeguard refusal (category: {category})."),
        model: Some(model.to_owned()),
        provider: Some("anthropic".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    })))
}

fn non_empty_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) if !text.is_empty() => Some(text),
        Json::Text(_)
        | Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

fn mapping_text<'container>(container: &'container Json, key: &str) -> Option<&'container str> {
    match container {
        Json::Object(fields) => fields.get(key).and_then(non_empty_text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn chat_refusal_text<'turn>(turn: &ChatTurn<'turn>) -> Option<&'turn str> {
    if let Some(text) = non_empty_text(turn.refusal) {
        return Some(text);
    }
    mapping_text(turn.provider_specific_fields, "refusal")
}

#[must_use]
pub fn responses_output_refusal_text<'item>(output: &[OutputItem<'item>]) -> Option<&'item str> {
    output.iter().find_map(|item| match item {
        OutputItem::Typed(SdkOutput::OutputMessage(message)) => {
            message.content.iter().find_map(|part| match part {
                OutputMessageContentValue::OutputRefusal(refusal) => {
                    Some(refusal.refusal.as_str()).filter(|text| !text.is_empty())
                }
                OutputMessageContentValue::OutputText(_) => None,
            })
        }
        OutputItem::Raw(raw) if mapping_text(raw, "type") == Some("message") => {
            let Json::Object(fields) = raw else {
                return None;
            };
            let Some(Json::List(parts)) = fields.get("content") else {
                return None;
            };
            parts.iter().find_map(|part| {
                if mapping_text(part, "type") == Some("refusal") {
                    mapping_text(part, "refusal")
                } else {
                    None
                }
            })
        }
        OutputItem::Raw(_) | OutputItem::Typed(_) => None,
    })
}
