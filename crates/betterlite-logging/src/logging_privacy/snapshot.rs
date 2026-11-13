use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;

use super::{credentials, request};
use indexmap::IndexMap;

const REDACTED: &str = "redacted-by-litellm";

#[must_use]
pub fn callback_snapshot(
    mut value: Json,
    decision: request::Decision,
    redactor: &Redactor,
) -> Json {
    if decision.content {
        redact_tree(&mut value);
    }
    credentials::scrub(&mut value, redactor);
    value
}

fn redact_tree(value: &mut Json) {
    match value {
        Json::Object(fields) => {
            scrub_provider_metadata(fields);
            if fields.contains_key("response") {
                fields.entry("messages".to_owned()).or_insert(Json::Null);
                fields.entry("prompt".to_owned()).or_insert(Json::Null);
                fields.entry("input".to_owned()).or_insert(Json::Null);
            }
            for (key, child) in fields.iter_mut() {
                match key.as_str() {
                    "response" => redact_result(child),
                    "messages" => {
                        *child = Json::List(vec![Json::Object(IndexMap::from([
                            ("role".to_owned(), Json::Text("user".to_owned())),
                            ("content".to_owned(), Json::Text(REDACTED.to_owned())),
                        ]))]);
                    }
                    "prompt" | "input" => *child = Json::Text(String::new()),
                    "content" | "reasoning_content" | "text" | "refusal" | "arguments"
                        if !matches!(child, Json::Null) =>
                    {
                        *child = Json::Text(REDACTED.to_owned());
                    }
                    "thinking_blocks" | "audio" | "logprobs" | "annotations" => *child = Json::Null,
                    _ => redact_tree(child),
                }
            }
        }
        Json::List(items) => items.iter_mut().for_each(redact_tree),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

fn redact_result(value: &mut Json) {
    match value {
        Json::Null => {}
        Json::Object(fields)
            if fields.contains_key("choices")
                || fields.contains_key("output")
                || fields.get("object") == Some(&Json::Text("list".to_owned())) =>
        {
            redact_tree(value);
        }
        Json::Object(_) | Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            *value = Json::Object(IndexMap::from([(
                "text".to_owned(),
                Json::Text(REDACTED.to_owned()),
            )]));
        }
    }
}

fn scrub_provider_metadata(fields: &mut IndexMap<String, Json>) {
    if fields.get("object") == Some(&Json::Text("list".to_owned())) && fields.contains_key("data") {
        fields.insert("data".to_owned(), Json::List(Vec::new()));
    }
    for field in [
        "served_output_texts",
        "classifier_input",
        "originating_request_masked",
    ] {
        fields.shift_remove(field);
    }
    for field in [
        "vertex_ai_grounding_metadata",
        "vertex_ai_url_context_metadata",
        "vertex_ai_safety_ratings",
        "vertex_ai_safety_results",
        "vertex_ai_citation_metadata",
    ] {
        if let Some(value) = fields.get_mut(field) {
            *value = Json::List(Vec::new());
        }
        for hidden in ["_hidden_params", "hidden_params"] {
            if let Some(Json::Object(params)) = fields.get_mut(hidden) {
                params.shift_remove(field);
            }
        }
    }
}

#[must_use]
pub fn redact_user_api_key_info(metadata: &Json, enabled: bool) -> Json {
    if !enabled {
        return metadata.clone();
    }
    match metadata {
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .filter(|(key, _)| !key.starts_with("user_api_key"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            metadata.clone()
        }
    }
}
