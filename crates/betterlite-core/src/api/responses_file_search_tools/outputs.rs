use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::unions::{
    OutputMessageContentValue, SdkOutput,
};
use betterlite_openai::api::responses_shapes::GenericResponseOutputItem;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use serde::Serialize;

use super::error::FileSearchToolsError;
use super::results::{SearchResultRecord, include_record};

pub const COMPLETED: &str = "completed";
const FILE_SEARCH_CALL_TYPE: &str = "file_search_call";
const OUTPUT_TEXT_TYPE: &str = "output_text";
const MESSAGE_TYPE: &str = "message";
const FILE_CITATION_TYPE: &str = "file_citation";

// the file_search_call output item mirrors OpenAI's format, with search_results only when the caller asked to include them
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileSearchCallOutput {
    pub r#type: &'static str,
    pub id: String,
    pub status: &'static str,
    pub queries: Vec<String>,
    pub search_results: Option<Vec<SearchResultRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCitation {
    pub r#type: &'static str,
    pub index: usize,
    pub file_id: String,
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextContent {
    pub r#type: &'static str,
    pub text: String,
    pub annotations: Vec<FileCitation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageOutput {
    pub r#type: &'static str,
    pub role: &'static str,
    pub content: Vec<TextContent>,
}

pub fn build_file_search_call_output(
    call_id: &str,
    queries: &[String],
    results: Option<&[VectorStoreSearchResult]>,
    include_search_results: bool,
) -> Result<FileSearchCallOutput, FileSearchToolsError> {
    let search_results = match results {
        Some(results) if include_search_results && !results.is_empty() => {
            let records = results
                .iter()
                .map(include_record)
                .collect::<Result<Vec<_>, _>>()?;
            Some(records)
        }
        None | Some(_) => None,
    };
    Ok(FileSearchCallOutput {
        r#type: FILE_SEARCH_CALL_TYPE,
        id: call_id.to_owned(),
        status: COMPLETED,
        queries: queries.to_vec(),
        search_results,
    })
}

// each result with a first-seen file_id is cited once, at the end of the text block
#[must_use]
pub fn citation_annotations(results: &[VectorStoreSearchResult], text: &str) -> Vec<FileCitation> {
    let index = text.chars().count();
    let mut seen: Vec<&str> = Vec::new();
    let mut annotations = Vec::new();
    for result in results {
        let Some(file_id) = result.file_id.as_deref().filter(|id| !id.is_empty()) else {
            continue;
        };
        if seen.contains(&file_id) {
            continue;
        }
        seen.push(file_id);
        annotations.push(FileCitation {
            r#type: FILE_CITATION_TYPE,
            index,
            file_id: file_id.to_owned(),
            filename: result.filename.clone().unwrap_or_default(),
        });
    }
    annotations
}

#[must_use]
pub fn build_message_output(text: &str, results: &[VectorStoreSearchResult]) -> MessageOutput {
    MessageOutput {
        r#type: MESSAGE_TYPE,
        role: "assistant",
        content: vec![TextContent {
            r#type: OUTPUT_TEXT_TYPE,
            text: text.to_owned(),
            annotations: citation_annotations(results, text),
        }],
    }
}

// the first output_text block of the first message item wins; a null or numeric text becomes "" or str(text)
pub fn response_output_text(output: &[ResponseOutputItem]) -> Result<String, FileSearchToolsError> {
    for item in output {
        if let Some(text) = message_text(item)? {
            return Ok(text);
        }
    }
    Ok(String::new())
}

fn message_text(item: &ResponseOutputItem) -> Result<Option<String>, FileSearchToolsError> {
    match item {
        ResponseOutputItem::Sdk(output) => Ok(sdk_message_text(output)),
        ResponseOutputItem::Message(message) => Ok(typed_message_text(message)),
        ResponseOutputItem::Unknown(fields) => unknown_message_text(fields),
        ResponseOutputItem::FunctionCall(_)
        | ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => Ok(None),
    }
}

fn sdk_message_text(output: &SdkOutput) -> Option<String> {
    let SdkOutput::OutputMessage(message) = output else {
        return None;
    };
    message.content.iter().find_map(|part| match part {
        OutputMessageContentValue::OutputText(text) => Some(text.text.clone()),
        OutputMessageContentValue::OutputRefusal(_) => None,
    })
}

fn typed_message_text(message: &GenericResponseOutputItem) -> Option<String> {
    if message.r#type != MESSAGE_TYPE {
        return None;
    }
    message.content.iter().find_map(|part| {
        if part.r#type.as_deref() == Some(OUTPUT_TEXT_TYPE) {
            return Some(part.text.clone().unwrap_or_default());
        }
        None
    })
}

fn unknown_message_text(
    fields: &BTreeMap<String, Json>,
) -> Result<Option<String>, FileSearchToolsError> {
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == MESSAGE_TYPE) {
        return Ok(None);
    }
    let content = match fields.get("content") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::List(blocks)) => blocks,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(FileSearchToolsError::WrongType {
                field: "content",
                expected: "a list",
            });
        }
    };
    for block in content {
        let Json::Object(block) = block else {
            continue;
        };
        if !matches!(block.get("type"), Some(Json::Text(kind)) if kind == OUTPUT_TEXT_TYPE) {
            continue;
        }
        return match block.get("text") {
            None | Some(Json::Null) => Ok(Some(String::new())),
            Some(Json::Text(text)) => Ok(Some(text.clone())),
            Some(Json::Number(number)) => Ok(Some(number.to_string())),
            Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
                Err(FileSearchToolsError::WrongType {
                    field: "text",
                    expected: "a string or a number",
                })
            }
        };
    }
    Ok(None)
}
