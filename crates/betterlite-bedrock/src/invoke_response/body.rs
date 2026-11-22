use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_openai::chat::response::ChatChoice;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize as _;
use serde_json::ser::PrettyFormatter;
use thiserror::Error;

use crate::invoke_response::error::ContentReason;
use crate::legacy_models::mistral::{MistralOutputError, output_text as mistral_text};

const INDENT: &[u8] = b"    ";

#[derive(Debug, Clone, Copy)]
pub(super) enum Family {
    Cohere,
    Ai21,
    Generation,
    Mistral,
    Titan,
}

#[derive(Debug, Error)]
pub(super) enum FamilyError {
    #[error(transparent)]
    Mistral(#[from] MistralOutputError),
    #[error("the {field} field is missing or malformed")]
    Malformed { field: &'static str },
    #[error("the {field} field has a type its declared shape does not allow")]
    FieldType { field: &'static str },
    #[error("the response has no choices[0] to fill")]
    MissingChoice,
    #[error("the response body is not a JSON object")]
    NotObject,
}

// LiteLLM logs json.dumps(completion_response, indent=4, default=str)
pub(super) fn debug_log(record: &Json) {
    let mut bytes = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut bytes, PrettyFormatter::with_indent(INDENT));
    if record.serialize(&mut serializer).is_err() {
        return;
    }
    let Ok(rendered) = String::from_utf8(bytes) else {
        return;
    };
    tracing::debug!("bedrock invoke response {rendered}");
}

pub(super) fn family_output(
    record: &Json,
    family: Family,
    choices: &mut [ChatChoice],
) -> Result<Json, FamilyError> {
    let Json::Object(fields) = record else {
        return Err(FamilyError::NotObject);
    };
    match family {
        Family::Cohere => cohere(fields, choices),
        Family::Ai21 => ai21(fields),
        Family::Generation => Ok(field(fields, "generation", "generation")?.clone()),
        Family::Mistral => mistral(fields, choices),
        Family::Titan => titan(fields),
    }
}

fn field<'value>(
    fields: &'value Object,
    key: &str,
    name: &'static str,
) -> Result<&'value Json, FamilyError> {
    match fields.get(key) {
        Some(value) => Ok(value),
        None => Err(FamilyError::Malformed { field: name }),
    }
}

fn cohere(fields: &Object, choices: &mut [ChatChoice]) -> Result<Json, FamilyError> {
    if let Some(text) = fields.get("text") {
        return Ok(text.clone());
    }
    let Some(generations) = fields.get("generations") else {
        return Ok(Json::Null);
    };
    let first = first_record(generations, "generations")?;
    let text = field(first, "text", "generations.text")?.clone();
    let finish = field(first, "finish_reason", "generations.finish_reason")?;
    let mapped = mapped_finish_reason(finish)?;
    let choice = choices.first_mut().ok_or(FamilyError::MissingChoice)?;
    choice.finish_reason = Some(mapped);
    Ok(text)
}

fn mapped_finish_reason(value: &Json) -> Result<String, FamilyError> {
    match value {
        Json::Text(text) => Ok(map_finish_reason(text).reason.as_str().to_owned()),
        Json::Null | Json::Bool(_) | Json::Number(_) => {
            Ok(map_finish_reason("stop").reason.as_str().to_owned())
        }
        Json::List(_) | Json::Object(_) => Err(FamilyError::FieldType {
            field: "generations.finish_reason",
        }),
    }
}

fn ai21(fields: &Object) -> Result<Json, FamilyError> {
    let completions = field(fields, "completions", "completions")?;
    let first = first_record(completions, "completions")?;
    let data = first
        .get("data")
        .and_then(Json::as_object)
        .ok_or(FamilyError::Malformed {
            field: "completions.data",
        })?;
    Ok(data.get("text").cloned().unwrap_or(Json::Null))
}

fn mistral(fields: &Object, choices: &mut [ChatChoice]) -> Result<Json, FamilyError> {
    let output = mistral_text(fields)?;
    let choice = choices.first_mut().ok_or(FamilyError::MissingChoice)?;
    choice.finish_reason = match output.finish_reason {
        Json::Text(text) => Some(text),
        Json::Null => None,
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(FamilyError::FieldType {
                field: "finish_reason",
            });
        }
    };
    Ok(output.text)
}

fn titan(fields: &Object) -> Result<Json, FamilyError> {
    let results = field(fields, "results", "results")?;
    let first = first_record(results, "results")?;
    Ok(first.get("outputText").cloned().unwrap_or(Json::Null))
}

fn first_record<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value Object, FamilyError> {
    let Json::List(items) = value else {
        return Err(FamilyError::Malformed { field });
    };
    items
        .first()
        .and_then(Json::as_object)
        .ok_or(FamilyError::Malformed { field })
}

pub(super) fn assign_content(
    choices: &mut [ChatChoice],
    output: &Json,
) -> Result<(), ContentReason> {
    let text = match output {
        Json::Text(text) => Some(text),
        Json::Null => None,
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(ContentReason::NotText);
        }
    };
    let Some(choice) = choices.first_mut() else {
        return Err(ContentReason::NoChoice);
    };
    if choice.message.tool_calls.is_some() {
        return Ok(());
    }
    match text {
        Some(value) if !value.is_empty() => {
            choice.message.content = Some(value.clone());
            Ok(())
        }
        None | Some(_) => Err(ContentReason::Empty),
    }
}

// A non-string output is spelled as standard JSON, since Python's str form is not ported.
pub(super) fn output_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}
