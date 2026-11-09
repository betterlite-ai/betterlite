use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use mime::Mime;
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use crate::ocr_config::OCR_INLINE_MAX_BYTES;
use crate::ocr_config::OcrRequestContext;
use crate::ocr_config::connection::OcrConnection;
use crate::ocr_config::decode::decode_response;
use crate::ocr_document::InlineDocument;
use crate::ocr_error::{OcrError, OcrHeaderError, OcrTransportError};
use crate::ocr_http::read_response_bytes;

use super::Boundaries;
use super::REDUCTO_ID_PREFIX;
use super::request::reducto_id;
use super::url::complete_endpoint_url;

#[derive(Debug, Deserialize)]
struct ReductoUploadResponse {
    file_id: Option<String>,
}

pub(super) async fn ensure_file_id(
    source: &str,
    headers: &[(String, String)],
    context: OcrRequestContext<'_>,
    boundaries: Boundaries,
) -> Result<String, OcrError> {
    if source.starts_with(REDUCTO_ID_PREFIX) {
        return reducto_id(source);
    }
    let inline = InlineDocument::parse(source)?.ok_or(OcrError::ReductoSource)?;
    let mime = inline.0.mime_type().to_string();
    let bytes = inline.decode(OCR_INLINE_MAX_BYTES)?;
    let content_type: Mime = mime.parse().map_err(|_source| OcrError::InvalidDataUri)?;
    upload_bytes(bytes, content_type.as_ref(), headers, context, boundaries).await
}

async fn upload_bytes(
    bytes: Vec<u8>,
    mime: &str,
    headers: &[(String, String)],
    context: OcrRequestContext<'_>,
    boundaries: Boundaries,
) -> Result<String, OcrError> {
    let url = complete_endpoint_url(context.connection.api_base.as_deref(), "upload")?;
    let outbound = upload_request(Upload {
        connection: context.connection,
        url,
        headers,
        bytes,
        mime: mime.to_owned(),
        boundary: boundaries(),
    })?;
    let response = context
        .client
        .provider_http
        .send(outbound)
        .await
        .map_err(|error| OcrError::Transport(OcrTransportError::Failed(error)))?;
    let body = read_response_bytes(response, context.connection.max_response_bytes).await?;
    let upload: ReductoUploadResponse = decode_response(&body, false)?.data;
    upload
        .file_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or(OcrError::ResponseField {
            path: "file_id".to_owned(),
        })
}

struct Upload<'call> {
    connection: &'call OcrConnection,
    url: String,
    headers: &'call [(String, String)],
    bytes: Vec<u8>,
    mime: String,
    boundary: Uuid,
}

// LiteLLM's multipart part keeps the document under field "file" with the data URI's own type
fn upload_request(upload: Upload<'_>) -> Result<OutboundRequest, OcrError> {
    let parsed = Url::parse(&upload.url).map_err(|_source| OcrError::RequestField {
        path: "url".to_owned(),
    })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url: parsed,
        query: Vec::new(),
        headers: upload_headers(upload.headers)?,
        body: RequestBody::Multipart {
            boundary: upload.boundary.simple().to_string(),
            parts: vec![MultipartPart {
                name: "file".to_owned(),
                file_name: Some("document".to_owned()),
                content_type: Some(upload.mime),
                data: upload.bytes,
            }],
        },
        deadline: upload.connection.timeout,
    })
}

// the multipart encoder writes its own content-type and length, so a caller's pair is dropped
fn upload_headers(headers: &[(String, String)]) -> Result<HeaderMap, OcrError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") || name.eq_ignore_ascii_case("content-length")
        {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            OcrError::Headers(OcrHeaderError::Name {
                name: name.clone(),
                source,
            })
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|source| {
            OcrError::Headers(OcrHeaderError::Value {
                name: name.clone(),
                source,
            })
        })?;
        map.append(header_name, header_value);
    }
    Ok(map)
}
