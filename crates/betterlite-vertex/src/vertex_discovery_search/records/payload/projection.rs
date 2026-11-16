use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResult,
};
use indexmap::IndexMap;

use super::{Chunk, DerivedStructData, Document, ExtractiveContent, Hit, PayloadError, Snippet};

const UNKNOWN_DOCUMENT: &str = "Unknown Document";
const EXTRACTIVE_SEPARATOR: &str = "\n\n";
const SNIPPET_SEPARATOR: &str = " ";

fn result_with(
    score: Score,
    text: String,
    file_id: String,
    title: &str,
    attributes: IndexMap<String, Json>,
) -> Result<VectorStoreSearchResult, PayloadError> {
    Ok(VectorStoreSearchResult {
        score: Some(score),
        content: Some(vec![VectorStoreResultContent {
            text: Some(text),
            type_name: Some("text".to_owned()),
            extra: BTreeMap::new(),
        }]),
        file_id: Some(file_id),
        filename: Some(if title.is_empty() {
            UNKNOWN_DOCUMENT.to_owned()
        } else {
            title.to_owned()
        }),
        attributes: Some(serde_json::to_value(Json::Object(attributes))?),
        extra: BTreeMap::new(),
    })
}

// LiteLLM's _chunk_result: the chunk's document id comes from its resource name, not its metadata
pub(super) fn chunk_result(
    chunk: &Chunk,
    positional: Score,
) -> Result<VectorStoreSearchResult, PayloadError> {
    let metadata = chunk.document_metadata.as_ref();
    let uri = metadata
        .and_then(|details| details.uri.as_deref())
        .unwrap_or("");
    let title = metadata
        .and_then(|details| details.title.as_deref())
        .unwrap_or("");
    let document_id = document_id_from_chunk_name(chunk.name.as_deref().unwrap_or(""));
    let score = match &chunk.relevance_score {
        Some(number) => Score::try_from(number.clone()).map_err(|source| PayloadError::Score {
            field: "relevanceScore",
            source,
        })?,
        None => positional,
    };
    let file_id = if uri.is_empty() {
        document_id.clone()
    } else {
        uri.to_owned()
    };
    result_with(
        score,
        chunk.content.clone().unwrap_or_default(),
        file_id,
        title,
        chunk_attributes(chunk, &document_id, uri, title),
    )
}

fn chunk_attributes(
    chunk: &Chunk,
    document_id: &str,
    uri: &str,
    title: &str,
) -> IndexMap<String, Json> {
    let mut attributes =
        IndexMap::from([("document_id".to_owned(), Json::Text(document_id.to_owned()))]);
    let chunk_id = chunk.id.clone().unwrap_or_default();
    if !chunk_id.is_empty() {
        attributes.insert("chunk_id".to_owned(), Json::Text(chunk_id));
    }
    if !uri.is_empty() {
        attributes.insert("link".to_owned(), Json::Text(uri.to_owned()));
    }
    if !title.is_empty() {
        attributes.insert("title".to_owned(), Json::Text(title.to_owned()));
    }
    let struct_data = chunk
        .document_metadata
        .as_ref()
        .and_then(|metadata| metadata.struct_data.as_ref())
        .filter(|fields| !fields.is_empty());
    if let Some(fields) = struct_data {
        attributes.insert("structData".to_owned(), Json::Object(fields.clone()));
    }
    let page_span = chunk.page_span.as_ref().filter(|fields| !fields.is_empty());
    if let Some(span) = page_span {
        attributes.insert("pageSpan".to_owned(), Json::Object(span.clone()));
    }
    attributes
}

// LiteLLM's _document_id_from_chunk_name keeps the first path segment after /documents/
fn document_id_from_chunk_name(name: &str) -> String {
    let after = name
        .split_once("/documents/")
        .map(|(_prefix, after)| after)
        .unwrap_or_default();
    after.split('/').next().unwrap_or_default().to_owned()
}

// LiteLLM's _document_result: the link names the file, and structData stays a document attribute
pub(super) fn document_result(
    hit: &Hit,
    positional: Score,
) -> Result<VectorStoreSearchResult, PayloadError> {
    let document = hit.document.as_ref();
    let derived = document.and_then(|entry| entry.derived.as_ref());
    let link = derived
        .and_then(|fields| fields.link.as_deref())
        .unwrap_or("");
    let title = derived
        .and_then(|fields| fields.title.as_deref())
        .unwrap_or("");
    let document_id = hit.id.as_deref().unwrap_or("");
    let file_id = if link.is_empty() { document_id } else { link };
    result_with(
        positional,
        document_text(derived),
        file_id.to_owned(),
        title,
        document_attributes(document, derived, link, title, document_id),
    )
}

fn document_attributes(
    document: Option<&Document>,
    derived: Option<&DerivedStructData>,
    link: &str,
    title: &str,
    document_id: &str,
) -> IndexMap<String, Json> {
    let mut attributes =
        IndexMap::from([("document_id".to_owned(), Json::Text(document_id.to_owned()))]);
    let display_link = derived
        .and_then(|fields| fields.display_link.as_deref())
        .unwrap_or("");
    let formatted_url = derived
        .and_then(|fields| fields.formatted_url.as_deref())
        .unwrap_or("");
    for (name, value) in [
        ("link", link),
        ("title", title),
        ("displayLink", display_link),
        ("formattedUrl", formatted_url),
    ] {
        if !value.is_empty() {
            attributes.insert(name.to_owned(), Json::Text(value.to_owned()));
        }
    }
    let struct_data = document
        .and_then(|entry| entry.struct_data.as_ref())
        .filter(|fields| !fields.is_empty());
    if let Some(fields) = struct_data {
        attributes.insert("structData".to_owned(), Json::Object(fields.clone()));
    }
    attributes
}

fn document_text(derived: Option<&DerivedStructData>) -> String {
    let Some(fields) = derived else {
        return String::new();
    };
    let segments = joined_content(fields.extractive_segments.as_deref().unwrap_or_default());
    if !segments.is_empty() {
        return segments;
    }
    let answers = joined_content(fields.extractive_answers.as_deref().unwrap_or_default());
    if !answers.is_empty() {
        return answers;
    }
    let snippets = snippet_text(fields.snippets.as_deref().unwrap_or_default());
    if !snippets.is_empty() {
        return snippets;
    }
    fields.title.clone().unwrap_or_default()
}

fn joined_content(entries: &[ExtractiveContent]) -> String {
    entries
        .iter()
        .filter_map(|entry| entry.content.as_deref())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join(EXTRACTIVE_SEPARATOR)
}

// LiteLLM's _snippet_text: a present snippet wins even when it is empty, and htmlSnippet is the fallback
fn snippet_text(snippets: &[Snippet]) -> String {
    snippets
        .iter()
        .map(|snippet| {
            snippet
                .snippet
                .as_deref()
                .or(snippet.html.as_deref())
                .unwrap_or("")
        })
        .collect::<Vec<_>>()
        .join(SNIPPET_SEPARATOR)
}
