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
        "embedding" | "aembedding" => value_messages(request, "input"),
        "image_generation" | "aimage_generation" | "atext_completion" | "text_completion" => {
            required_messages(request, "prompt")
        }
        "moderation" | "amoderation" => required_messages(request, "input"),
        "rerank" | "arerank" => value_messages(request, "query"),
        "search" | "asearch" => search_messages(request),
        "decisions" | "adecisions" | "systemone" | "asystemone" => decision_messages(request),
        "image_edit" | "aimage_edit" => value_messages(request, "prompt"),
        "ocr" | "aocr" => Ok(Some(Messages::Text(ocr_document_summary(
            request.get("document"),
            None,
        )))),
        "atranscription" | "transcription" => transcription_messages(request),
        "aspeech" | "speech" => speech_messages(request),
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
    let source = match request.get("messages").cloned() {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::List(messages)) => messages,
        Some(_) => {
            return Err(CallSetupError::FieldType {
                field: "messages",
                expected: "a list of messages",
            });
        }
    };
    pre_call_rules(&source, model, rules)?;
    strip_thought_signatures(request, model);
    Ok(Some(Messages::Values(source)))
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

// The provider resolution for a bare model belongs to B174; until then only an explicit provider decides
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

fn value_messages(
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
    if request
        .get(field)
        .is_none_or(|value| matches!(value, Json::Null))
    {
        return Err(CallSetupError::MissingField { field });
    }
    value_messages(request, field)
}

fn search_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    match request.get("query") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(Messages::Text(text.clone()))),
        Some(Json::List(parts)) => {
            let mut joined = String::new();
            for (index, part) in parts.iter().enumerate() {
                let Json::Text(text) = part else {
                    return Err(CallSetupError::FieldType {
                        field: "query",
                        expected: "a list of strings",
                    });
                };
                if index > 0 {
                    joined.push('\n');
                }
                joined.push_str(text);
            }
            Ok(Some(Messages::Text(joined)))
        }
        Some(_) => Err(CallSetupError::FieldType {
            field: "query",
            expected: "a string or a list",
        }),
    }
}

fn decision_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    let selected = ["state", "input"].iter().find_map(|field| {
        request
            .get(*field)
            .filter(|value| !matches!(value, Json::Null))
    });
    match selected {
        None => Ok(Some(Messages::Text(String::new()))),
        Some(Json::Text(text)) => Ok(Some(Messages::Text(text.clone()))),
        Some(value) => Ok(Some(Messages::Text(serde_json::to_string(value)?))),
    }
}

fn transcription_messages(request: &mut RequestMap) -> Result<Option<Messages>, CallSetupError> {
    let file = match request.get("file").cloned() {
        None | Some(Json::Null) => return Err(CallSetupError::MissingField { field: "file" }),
        Some(Json::Text(file)) => file,
        Some(_) => {
            return Err(CallSetupError::FieldType {
                field: "file",
                expected: "a string",
            });
        }
    };
    let checksum = hex::encode(Sha256::digest(file.as_bytes()));
    set_file_checksum(request, &checksum)?;
    Ok(Some(Messages::Text(checksum)))
}

fn set_file_checksum(request: &mut RequestMap, checksum: &str) -> Result<(), CallSetupError> {
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
    metadata.insert(
        String::from("file_checksum"),
        Json::Text(checksum.to_owned()),
    );
    request.insert(String::from("metadata"), Json::Object(metadata));
    Ok(())
}

fn speech_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    if request
        .get("input")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return value_messages(request, "input");
    }
    Ok(Some(Messages::Text(String::from("speech"))))
}

fn responses_messages(request: &RequestMap) -> Result<Option<Messages>, CallSetupError> {
    if request
        .get("input")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return value_messages(request, "input");
    }
    if request
        .get("messages")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return value_messages(request, "messages");
    }
    Ok(Some(Messages::Text(DEFAULT_MESSAGE_VALUE.to_owned())))
}
