use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

pub const REDACTED_TEXT: &str = "redacted-by-litellm";

const CLASSIFIER_AUDIT_FIELDS: [&str; 2] = ["classifier_input", "originating_request_masked"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCallDetails {
    pub standard_logging_object: Option<IndexMap<String, Json>>,
    pub proxy_server_request: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionPolicy {
    pub turn_off_message_logging: bool,
    pub excluded_fields: Vec<String>,
    pub callback_redacts_itself: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseStamp {
    pub id: String,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redaction {
    Unchanged,
    Redacted {
        standard_logging_object: IndexMap<String, Json>,
        proxy_server_request: Option<IndexMap<String, Json>>,
    },
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn redact_standard_logging_payload(
    details: &ModelCallDetails,
    policy: &RedactionPolicy,
    stamp: &ResponseStamp,
) -> Redaction {
    if !policy.turn_off_message_logging && policy.excluded_fields.is_empty() {
        return Redaction::Unchanged;
    }
    let Some(standard_logging_object) = details.standard_logging_object.as_ref() else {
        return Redaction::Unchanged;
    };
    let mut copy: IndexMap<String, Json> = standard_logging_object
        .iter()
        .filter(|(key, _)| {
            !(policy
                .excluded_fields
                .iter()
                .any(|field| field.as_str() == key.as_str())
                || policy.turn_off_message_logging
                    && CLASSIFIER_AUDIT_FIELDS.contains(&key.as_str()))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if policy.turn_off_message_logging && !policy.callback_redacts_itself {
        redact_messages(&mut copy);
        redact_response(&mut copy, stamp);
    }
    let proxy_server_request = match (
        &details.proxy_server_request,
        policy.turn_off_message_logging,
    ) {
        (Some(request), true) => Some(
            request
                .iter()
                .filter(|(key, _)| !CLASSIFIER_AUDIT_FIELDS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        (request, _) => request.clone(),
    };
    Redaction::Redacted {
        standard_logging_object: copy,
        proxy_server_request,
    }
}

fn redact_messages(object: &mut IndexMap<String, Json>) {
    if matches!(object.get("messages"), None | Some(Json::Null)) {
        return;
    }
    object.insert(
        "messages".to_owned(),
        Json::List(vec![Json::Object(message_placeholder())]),
    );
}

fn redact_response(object: &mut IndexMap<String, Json>, stamp: &ResponseStamp) {
    let Some(response) = object.get("response") else {
        return;
    };
    if matches!(response, Json::Null) {
        return;
    }
    let redacted = match response {
        Json::Object(fields) if fields.contains_key("output") => {
            let mut copy = fields.clone();
            redact_output_text(&mut copy);
            Json::Object(copy)
        }
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => Json::Object(model_response_placeholder(stamp)),
    };
    object.insert("response".to_owned(), redacted);
}

fn redact_output_text(fields: &mut IndexMap<String, Json>) {
    let Some(Json::List(items)) = fields.get_mut("output") else {
        return;
    };
    for item in items {
        let Json::Object(item_fields) = item else {
            continue;
        };
        let Some(Json::List(content)) = item_fields.get_mut("content") else {
            continue;
        };
        for part in content {
            let Json::Object(part_fields) = part else {
                continue;
            };
            if part_fields.contains_key("text") {
                part_fields.insert("text".to_owned(), Json::Text(REDACTED_TEXT.to_owned()));
            }
        }
    }
}

fn message_placeholder() -> IndexMap<String, Json> {
    IndexMap::from([
        ("content".to_owned(), Json::Text(REDACTED_TEXT.to_owned())),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("tool_calls".to_owned(), Json::Null),
        ("function_call".to_owned(), Json::Null),
        ("provider_specific_fields".to_owned(), Json::Null),
    ])
}

fn model_response_placeholder(stamp: &ResponseStamp) -> IndexMap<String, Json> {
    let message = IndexMap::from([
        ("content".to_owned(), Json::Text(REDACTED_TEXT.to_owned())),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("tool_calls".to_owned(), Json::Null),
        ("function_call".to_owned(), Json::Null),
    ]);
    let choice = IndexMap::from([
        ("finish_reason".to_owned(), Json::Text("stop".to_owned())),
        ("index".to_owned(), Json::Number(Number::from(0_i64))),
        ("message".to_owned(), Json::Object(message)),
    ]);
    IndexMap::from([
        ("id".to_owned(), Json::Text(stamp.id.clone())),
        (
            "created".to_owned(),
            Json::Number(Number::from(stamp.created)),
        ),
        ("model".to_owned(), Json::Null),
        (
            "object".to_owned(),
            Json::Text("chat.completion".to_owned()),
        ),
        ("system_fingerprint".to_owned(), Json::Null),
        ("choices".to_owned(), Json::List(vec![Json::Object(choice)])),
    ])
}
