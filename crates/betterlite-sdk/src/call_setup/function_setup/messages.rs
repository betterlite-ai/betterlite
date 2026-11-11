use betterlite_core::message_utilities::thought_signatures::is_gemini_model;
use betterlite_core::message_utilities::thought_signatures::strip_signatures_from_messages;
use betterlite_core::rules::RuleEvaluator;
use betterlite_core::rules::RulePhase;
use betterlite_core::rules::evaluate;
use betterlite_logging::logging_state::types::Messages;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::Digest as _;
use sha2::Sha256;

use super::CallSetupError;
use crate::call_setup::RequestMap;
use crate::call_setup::text::ocr::ocr_document_summary;

const DEFAULT_MESSAGE_VALUE: &str = "default-message-value";
const THOUGHT_SIGNATURE_SEPARATOR: &str = "__thought__";

// The call types LiteLLM logs with an empty message list instead of a default text
const NON_INFERENCE_CALL_TYPES: [&str; 30] = [
    "get_responses",
    "aget_responses",
    "delete_responses",
    "adelete_responses",
    "cancel_responses",
    "acancel_responses",
    "list_input_items",
    "alist_input_items",
    "vector_store_create",
    "avector_store_create",
    "vector_store_retrieve",
    "avector_store_retrieve",
    "vector_store_list",
    "avector_store_list",
    "vector_store_update",
    "avector_store_update",
    "vector_store_delete",
    "avector_store_delete",
    "vector_store_file_create",
    "avector_store_file_create",
    "vector_store_file_list",
    "avector_store_file_list",
    "vector_store_file_retrieve",
    "avector_store_file_retrieve",
    "vector_store_file_content",
    "avector_store_file_content",
    "vector_store_file_update",
    "avector_store_file_update",
    "vector_store_file_delete",
    "avector_store_file_delete",
];

pub(super) fn message_source<Evaluator>(
    call_type: &str,
    request: &mut RequestMap,
    model: &str,
    rules: &Evaluator,
) -> Result<Option<Messages>, CallSetupError>
where
    Evaluator: RuleEvaluator,
{
    match call_type {
        "completion" | "acompletion" | "anthropic_messages" => chat_messages(request, model, rules),
        "embedding" | "aembedding" => field_messages(request, "input"),
        "rerank" | "arerank" => field_messages(request, "query"),
        "image_edit" | "aimage_edit" => field_messages(request, "prompt"),
        "image_generation" | "aimage_generation" | "atext_completion" | "text_completion" => {
            required_messages(request, "prompt")
        }
        "moderation" | "amoderation" => required_messages(request, "input"),
        "search" | "asearch" => search_messages(request),
        "decisions" | "adecisions" | "systemone" | "asystemone" => decision_messages(request),
        "atranscription" | "transcription" => Ok(Some(Messages::Text(file_checksum(request)?))),
        "ocr" | "aocr" => Ok(Some(Messages::Text(ocr_document_summary(
            request.get("document"),
            None,
        )))),
        "aspeech" | "speech" => Ok(Some(
            field_messages(request, "input")?.unwrap_or(Messages::Text(String::from("speech"))),
        )),
        "aresponses" | "responses" => responses_messages(request),
        "generate_content"
        | "agenerate_content"
        | "generate_content_stream"
        | "agenerate_content_stream" => Ok(Some(Messages::Text(DEFAULT_MESSAGE_VALUE.to_owned()))),
        other if NON_INFERENCE_CALL_TYPES.contains(&other) => {
            Ok(Some(Messages::Values(Vec::new())))
        }
        _ => Ok(Some(Messages::Text(DEFAULT_MESSAGE_VALUE.to_owned()))),
    }
}

fn chat_messages<Evaluator>(
    request: &mut RequestMap,
    model: &str,
    rules: &Evaluator,
) -> Result<Option<Messages>, CallSetupError>
where
    Evaluator: RuleEvaluator,
{
    let messages = field_messages(request, "messages")?;
    if let Some(Messages::Values(source)) = &messages {
        pre_call_rules(source, model, rules)?;
        strip_thought_signatures(request, model);
    }
    Ok(messages)
}

fn field_messages(
    request: &RequestMap,
    field: &'static str,
) -> Result<Option<Messages>, CallSetupError> {
    match request.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(Messages::Text(text.clone()))),
        Some(Json::List(values)) => Ok(Some(Messages::Values(values.clone()))),
        Some(_) => Err(CallSetupError::FieldType {
            field,
            expected: "a string or a list",
        }),
    }
}

fn required_messages(
    request: &RequestMap,
    field: &'static str,
) -> Result<Option<Messages>, CallSetupError> {
    field_messages(request, field)?
        .map(Some)
        .ok_or(CallSetupError::MissingField { field })
}

fn search_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    if let Some(Json::List(parts)) = request.get("query") {
        return joined_parts(parts);
    }
    field_messages(request, "query")
}

fn joined_parts(parts: &[Json]) -> Result<Option<Messages>, CallSetupError> {
    let texts = parts
        .iter()
        .map(|part| match part {
            Json::Text(text) => Ok(text.as_str()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(CallSetupError::FieldType {
                    field: "query",
                    expected: "a list of strings",
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(Messages::Text(texts.join("\n"))))
}

fn decision_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    match first_present_field(request, &["state", "input"]) {
        None => Ok(Some(Messages::Text(String::new()))),
        Some(Json::Text(text)) => Ok(Some(Messages::Text(text.clone()))),
        Some(value) => Ok(Some(Messages::Text(serde_json::to_string(value)?))),
    }
}

fn first_present_field<'request>(
    request: &'request RequestMap,
    fields: &[&str],
) -> Option<&'request Json> {
    fields.iter().find_map(|field| {
        request
            .get(*field)
            .filter(|value| !matches!(value, Json::Null))
    })
}

fn responses_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    let mut messages = field_messages(request, "input")?;
    if messages.is_none() {
        messages = field_messages(request, "messages")?;
    }
    Ok(Some(messages.unwrap_or(Messages::Text(
        DEFAULT_MESSAGE_VALUE.to_owned(),
    ))))
}

fn pre_call_rules<Evaluator>(
    messages: &[Json],
    model: &str,
    rules: &Evaluator,
) -> Result<(), CallSetupError>
where
    Evaluator: RuleEvaluator,
{
    let Some(Json::Object(first)) = messages.first() else {
        return Ok(());
    };
    if !first.contains_key("content") {
        return Ok(());
    }
    let mut buffer = String::new();
    for message in messages {
        let Json::Object(fields) = message else {
            return Err(CallSetupError::FieldType {
                field: "messages",
                expected: "a list of objects",
            });
        };
        if let Some(Json::Text(content)) = fields.get("content") {
            buffer.push_str(content);
        }
    }
    evaluate(RulePhase::PreCall, rules, Some(&buffer), model)?;
    Ok(())
}

fn strip_thought_signatures(request: &mut RequestMap, model: &str) {
    let provider = match request.get("custom_llm_provider") {
        Some(Json::Text(provider)) => Some(provider.as_str()),
        _ => None,
    };
    if is_gemini_model(Some(model), provider) {
        return;
    }
    let Some(Json::List(messages)) = request.get("messages").cloned() else {
        return;
    };
    match strip_signatures_from_messages(&messages, THOUGHT_SIGNATURE_SEPARATOR) {
        Ok(processed) => {
            request.insert(String::from("messages"), Json::List(processed));
        }
        Err(error) => tracing::warn!(%error, "the thought signatures were not removed"),
    }
}

fn file_checksum(request: &mut RequestMap) -> Result<String, CallSetupError> {
    let file = match field_messages(request, "file")? {
        Some(Messages::Text(file)) => file,
        None => return Err(CallSetupError::MissingField { field: "file" }),
        Some(Messages::Values(_)) => {
            return Err(CallSetupError::FieldType {
                field: "file",
                expected: "a string",
            });
        }
    };
    let checksum = hex::encode(Sha256::digest(file.as_bytes()));
    let mut metadata = match request.get("metadata").cloned() {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(fields)) => fields,
        Some(_) => {
            return Err(CallSetupError::FieldType {
                field: "metadata",
                expected: "an object",
            });
        }
    };
    metadata.insert(String::from("file_checksum"), Json::Text(checksum.clone()));
    request.insert(String::from("metadata"), Json::Object(metadata));
    Ok(checksum)
}
