use std::collections::BTreeMap;
use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::error::HttpError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
// ast-grep-ignore: no-long-names
use betterlite_vertex::vertex_batch_download::records::transform_file_content_response;
use betterlite_vertex::vertex_batch_download::records::{ContentResponse, ConversionContext};
use betterlite_vertex::vertex_batch_download::streaming::transform_file_content_stream;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use http::header::{CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Buffer {
        content: String,
        url: String,
        headers: BTreeMap<String, String>,
        disabled: bool,
        now: i64,
    },
    Stream {
        chunks: Vec<String>,
        url: String,
        headers: BTreeMap<String, String>,
        disabled: bool,
        now: i64,
    },
}

pub(crate) fn run(input: &Input) -> HelperResult {
    let (original, found) = match input {
        Input::Buffer {
            content,
            url,
            headers,
            disabled,
            now,
        } => {
            let original = STANDARD.decode(content).or_config_error()?;
            // ast-grep-ignore: no-long-names
            let found = transform_file_content_response(
                &original,
                url,
                &header_map(headers)?,
                *disabled,
                &mut context(*now),
            );
            (original, found)
        }
        Input::Stream {
            chunks,
            url,
            headers,
            disabled,
            now,
        } => {
            let parts = decoded_chunks(chunks)?;
            let original = parts.concat();
            let found = drive(*disabled, url, header_map(headers)?, parts, *now)?;
            (original, found)
        }
    };
    project(&original, &found.content, &found.headers)
}

fn decoded_chunks(chunks: &[String]) -> Result<Vec<Vec<u8>>, Box<HelperError>> {
    let mut parts = Vec::with_capacity(chunks.len());
    for encoded in chunks {
        parts.push(STANDARD.decode(encoded).or_config_error()?);
    }
    Ok(parts)
}

// the production entry is async and owns its source, so a current-thread runtime drives it here
fn drive(
    disabled: bool,
    url: &str,
    headers: HeaderMap,
    parts: Vec<Vec<u8>>,
    now: i64,
) -> Result<ContentResponse, Box<HelperError>> {
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    runtime.block_on(async move {
        let source: BoxStream<'static, Result<Vec<u8>, HttpError>> = Box::pin(stream::iter(
            parts.into_iter().map(Ok::<Vec<u8>, HttpError>),
        ));
        let result =
            transform_file_content_stream(source, url, headers, disabled, context(now)).await;
        let streamed = result.or_value_error()?;
        let mut content = Vec::new();
        let mut output = streamed.stream;
        while let Some(chunk) = output.next().await {
            content.extend_from_slice(&chunk.or_value_error()?);
        }
        Ok(ContentResponse {
            content,
            headers: streamed.headers,
        })
    })
}

fn context(now: i64) -> ConversionContext {
    let mut issued = 0_u128;
    ConversionContext {
        now,
        ids: Box::new(move || {
            issued = issued.saturating_add(1);
            Uuid::from_u128(issued)
        }),
    }
}

fn header_map(pairs: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).or_config_error()?,
            HeaderValue::from_str(value).or_config_error()?,
        );
    }
    Ok(headers)
}

fn project(original: &[u8], content: &[u8], headers: &HeaderMap) -> HelperResult {
    Ok(json!({
        "content": STANDARD.encode(content),
        "content_length": headers.get(CONTENT_LENGTH).and_then(|value| value.to_str().ok()),
        "headers": other_headers(headers),
        "rows": rows(original, content)?,
    }))
}

fn other_headers(headers: &HeaderMap) -> Value {
    let mut fields = serde_json::Map::new();
    for (name, value) in headers {
        if name.as_str() == "content-length" {
            continue;
        }
        fields.insert(
            name.as_str().to_owned(),
            Value::String(value.to_str().unwrap_or_default().to_owned()),
        );
    }
    Value::Object(fields)
}

// the original bytes mean the call passed the file through, so there is no row projection
fn rows(original: &[u8], content: &[u8]) -> Result<Value, Box<HelperError>> {
    if content == original {
        return Ok(Value::Null);
    }
    let text = from_utf8(content).or_value_error()?;
    let mut parsed = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        parsed.push(serde_json::from_str::<Value>(line.trim()).or_value_error()?);
    }
    Ok(Value::Array(parsed))
}
