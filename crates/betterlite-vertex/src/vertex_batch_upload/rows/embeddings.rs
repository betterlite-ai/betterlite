use std::error::Error as StdError;

use betterlite_models::decisions::text::decisions_text;
use betterlite_models::decisions::{DecisionsJson, Extra, Json as OutputJson};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::gemini_embedding_records::input::{EmbeddingInput, InputEntry, ResolvedFiles};
use crate::gemini_embedding_records::request::transform_embed_content;
use crate::gemini_options::error::decode;
use crate::gemini_request::error::encode;
use crate::vertex_batch_upload::line::BatchLine;
use crate::vertex_batch_upload::rows::error::RowError;

// Python's quote(custom_id, safe="") keeps only letters, digits and _.-~
const KEY_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

// one Vertex row per outer input element, so an inner array stays one combined request
pub fn rows<Mapped>(line: &BatchLine) -> Result<Vec<String>, RowError<Mapped>>
where
    Mapped: StdError + 'static,
{
    let Some(Json::Object(body)) = &line.body else {
        return Err(RowError::Field("body", "a JSON object"));
    };
    let input = match body.get("input") {
        None | Some(Json::Null) => return Err(RowError::MissingInput),
        Some(value) => decode::<EmbeddingInput>(value, "input")?,
    };
    let elements: Vec<EmbeddingInput> = match input {
        EmbeddingInput::Text(text) => vec![EmbeddingInput::Text(text)],
        EmbeddingInput::Entries(entries) => entries.into_iter().map(outer_element).collect(),
    };
    if elements.is_empty() {
        return Err(RowError::EmptyInput);
    }
    let optional = encode::<Object, Extra>(body)?;
    let total = elements.len();
    let mut texts = Vec::with_capacity(total);
    for (index, element) in elements.iter().enumerate() {
        let mut row: IndexMap<String, OutputJson> = IndexMap::new();
        if let Some(custom_id) = &line.custom_id {
            let key = batch_key(custom_id, index, total);
            row.insert("key".to_owned(), OutputJson::Text(key));
        }
        row.insert(
            "request".to_owned(),
            embed_request::<Mapped>(element, &optional)?,
        );
        texts.push(decisions_text(&DecisionsJson::Object(row)).map_err(RowError::Dump)?);
    }
    Ok(texts)
}

// only a fan-out carries the position, and the id is escaped first so its own `#0/2` stays inert
fn batch_key(custom_id: &str, index: usize, total: usize) -> String {
    let encoded = utf8_percent_encode(custom_id, KEY_ENCODE).to_string();
    if total < 2 {
        return encoded;
    }
    format!("{encoded}#{index}/{total}")
}

fn outer_element(entry: InputEntry) -> EmbeddingInput {
    match entry {
        InputEntry::Text(text) => EmbeddingInput::Text(text),
        InputEntry::Group(group) => {
            EmbeddingInput::Entries(group.into_iter().map(InputEntry::Text).collect())
        }
    }
}

// the batch row names the config fields in snake case, and the model stays out of the request
fn embed_request<Mapped>(
    element: &EmbeddingInput,
    optional: &Extra,
) -> Result<OutputJson, RowError<Mapped>>
where
    Mapped: StdError + 'static,
{
    let embed = transform_embed_content(element, optional, &ResolvedFiles::new())?;
    let mut request: IndexMap<String, OutputJson> = IndexMap::new();
    request.insert("content".to_owned(), encode(&embed.content)?);
    if let Some(dimensions) = embed.parameters.output_dimensionality {
        request.insert(
            "output_dimensionality".to_owned(),
            OutputJson::Number(dimensions),
        );
    }
    if let Some(task_type) = embed.parameters.task_type {
        request.insert("task_type".to_owned(), OutputJson::Text(task_type));
    }
    if let Some(title) = embed.parameters.title {
        request.insert("title".to_owned(), OutputJson::Text(title));
    }
    Ok(OutputJson::Object(request))
}
