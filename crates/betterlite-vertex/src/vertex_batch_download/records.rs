use std::str::from_utf8;

use http::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};
use uuid::Uuid;

use crate::vertex_batch_download::policy::{DownloadPolicy, download_policy};
use crate::vertex_batch_rows::decode::{
    decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
};
use crate::vertex_batch_rows::embeddings::regroup_embeddings;
use crate::vertex_batch_rows::paths::model_from_managed_gcs_url;

pub(crate) mod body;
pub(crate) mod error;
pub(crate) mod row;
mod tests;

pub struct ConversionContext {
    pub now: i64,
    pub ids: Box<dyn FnMut() -> Uuid + Send>,
}

#[derive(Debug)]
pub struct ContentResponse {
    pub content: Vec<u8>,
    pub headers: HeaderMap,
}

// LiteLLM converts while the whole file is in memory and hands the original bytes back on any failure
#[must_use]
// ast-grep-ignore: no-long-names
pub fn transform_file_content_response(
    content: &[u8],
    request_url: &str,
    headers: &HeaderMap,
    disabled: bool,
    state: &mut ConversionContext,
) -> ContentResponse {
    let mut converted_headers = headers.clone();
    if download_policy(disabled, request_url) == DownloadPolicy::Convert
        && let Some(converted) = convert_whole(content, request_url, state)
        && converted != content
    {
        converted_headers.insert(CONTENT_LENGTH, HeaderValue::from(converted.len()));
        return ContentResponse {
            content: converted,
            headers: converted_headers,
        };
    }
    ContentResponse {
        content: content.to_vec(),
        headers: converted_headers,
    }
}

fn convert_whole(
    content: &[u8],
    request_url: &str,
    state: &mut ConversionContext,
) -> Option<Vec<u8>> {
    let lines = jsonl_lines(content)?;
    let first = lines.first()?;
    let row = decode_vertex_batch_row(first.as_bytes()).ok()?;
    if is_embeddings_output_row(&row) {
        let model = model_from_managed_gcs_url(request_url);
        return embeddings_content(content, model.as_deref(), state);
    }
    if !is_generate_output_row(&row) {
        return None;
    }
    generate_content(&lines, state)
}

fn generate_content(lines: &[&str], context: &mut ConversionContext) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let transformed = row::transform_line(line.as_bytes(), context).ok()?;
        if index > 0 {
            output.push(b'\n');
        }
        output.extend_from_slice(&transformed);
    }
    Some(output)
}

// the streamed embeddings path reads the whole file and regroups it exactly as the buffered one
pub(crate) fn embeddings_content(
    content: &[u8],
    model: Option<&str>,
    state: &mut ConversionContext,
) -> Option<Vec<u8>> {
    let lines = jsonl_lines(content)?;
    let mut rows = Vec::with_capacity(lines.len());
    for line in &lines {
        rows.push(decode_vertex_batch_row(line.as_bytes()).ok()?);
    }
    let outputs = regroup_embeddings(&rows, model, state.ids.as_mut()).ok()?;
    let mut output = Vec::new();
    for (index, row) in outputs.iter().enumerate() {
        if index > 0 {
            output.push(b'\n');
        }
        let encoded = serde_json::to_vec(row).ok()?;
        output.extend_from_slice(&encoded);
    }
    Some(output)
}

fn jsonl_lines(content: &[u8]) -> Option<Vec<&str>> {
    let mut lines = Vec::new();
    for raw in content.split(|byte| *byte == b'\n') {
        let text = from_utf8(raw).ok()?.trim();
        if !text.is_empty() {
            lines.push(text);
        }
    }
    Some(lines)
}
