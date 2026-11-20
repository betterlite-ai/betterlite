use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use secrecy::SecretString;
use serde_json::Number;
use url::Url;
use uuid::Uuid;

use super::config::{Headers, Params, RequestFile, RequestFiles};
use super::error::{FieldTypeError, WireFailure};
use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap as ErrorHeaders, ResponseFacts, header_text,
};
use crate::api::vector_store_http_write::service::WriteRequest;

// httpx's raise_for_status raises for any status from 400 up
pub(crate) const SUCCESS_LIMIT: u16 = 400;

// The litellm_params fields the handlers read
pub(crate) struct VideoParams {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) ssl_verify: Option<bool>,
}

impl VideoParams {
    // api_base and api_key are LiteLLM strings and ssl_verify a boolean; a wrong type is the caller's error
    pub(crate) fn read(params: &Params) -> Result<Self, FieldTypeError> {
        Ok(Self {
            api_base: text(params, "api_base")?.map(ToOwned::to_owned),
            api_key: text(params, "api_key")?.map(|key| SecretString::from(key.to_owned())),
            ssl_verify: flag(params, "ssl_verify")?,
        })
    }
}

fn text(params: &Params, field: &'static str) -> Result<Option<&str>, FieldTypeError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(string_type(field)),
    }
}

fn flag(params: &Params, field: &'static str) -> Result<Option<bool>, FieldTypeError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(flag_type(field)),
    }
}

const fn string_type(field: &'static str) -> FieldTypeError {
    FieldTypeError {
        field,
        expected: "a string",
    }
}

const fn flag_type(field: &'static str) -> FieldTypeError {
    FieldTypeError {
        field,
        expected: "a boolean",
    }
}

// An exception that carries no response: _handle_error synthesizes a 500 and uses the exception's text
pub(crate) fn display_facts(text: &str) -> FailureFacts {
    FailureFacts {
        response_status: None,
        status: None,
        text: None,
        headers: None,
        response: None,
        display: text.to_owned(),
    }
}

// An httpx.HTTPStatusError carries the response: its status, its text and its headers with content-length recomputed
pub(crate) fn status_facts(reply: &UpstreamResponse) -> FailureFacts {
    let text = from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        ToOwned::to_owned,
    );
    let mut headers = text_headers(&reply.headers);
    if !reply.body.is_empty() {
        headers.insert(
            "content-length".to_owned(),
            Json::Text(reply.body.len().to_string()),
        );
    }
    FailureFacts {
        response_status: Some(i64::from(reply.status)),
        status: Some(Json::Number(Number::from(reply.status))),
        text: Some(text.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(reply.status),
            text: Some(text),
            headers: Some(headers),
        }),
        display: String::new(),
    }
}

fn text_headers(headers: &HttpHeaders) -> ErrorHeaders {
    headers
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "content-encoding" | "content-length"))
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect()
}

// get_error_class takes typed name/value pairs, and every header the converter carries is text
pub(crate) fn header_pairs(headers: &ErrorHeaders) -> Headers {
    let mut pairs = Headers::new();
    for (name, value) in headers {
        if let Json::Text(text) = value {
            pairs.insert(name.clone(), text.clone());
        }
    }
    pairs
}

// Files send multipart, and no files (or an empty list) send the JSON body as LiteLLM does
pub(crate) fn video_request(
    url: &str,
    headers: &Headers,
    data: &Params,
    files: Option<&RequestFiles>,
    timeout: Option<Duration>,
) -> Result<WriteRequest, WireFailure> {
    let body = match files {
        Some(upload_files) => {
            let parts = multipart_parts(data, upload_files)?;
            let boundary = Uuid::new_v4().simple().to_string();
            RequestBody::Multipart { boundary, parts }
        }
        None => RequestBody::Json(serde_json::to_vec(data).map_err(WireFailure::Encode)?),
    };
    Ok(WriteRequest {
        method: Method::POST,
        url: Url::parse(url).map_err(|source| WireFailure::RequestUrl {
            url: url.to_owned(),
            source,
        })?,
        headers: wire_headers(headers)?,
        body,
        timeout,
    })
}

// httpx renders the data fields first and the file fields after them, in the order it was given
fn multipart_parts(
    data: &Params,
    files: &[RequestFile],
) -> Result<Vec<MultipartPart>, WireFailure> {
    let mut parts = Vec::new();
    for (name, value) in data {
        if let Json::List(items) = value {
            for item in items {
                parts.push(field_part(name, item)?);
            }
        } else {
            parts.push(field_part(name, value)?);
        }
    }
    for file in files {
        parts.push(file_part(file));
    }
    Ok(parts)
}

fn file_part(file: &RequestFile) -> MultipartPart {
    MultipartPart {
        name: file.field.clone(),
        file_name: file.filename.clone(),
        content_type: file.content_type.clone(),
        data: file.content.clone(),
    }
}

// primitive_value_to_str: true and false as JSON spells them, None as the empty string, numbers as str() does
fn field_part(name: &str, value: &Json) -> Result<MultipartPart, WireFailure> {
    let text = match value {
        Json::Null => String::new(),
        Json::Bool(true) => "true".to_owned(),
        Json::Bool(false) => "false".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => {
            return Err(WireFailure::Param {
                field: name.to_owned(),
            });
        }
    };
    Ok(MultipartPart {
        name: name.to_owned(),
        file_name: None,
        content_type: None,
        data: text.into_bytes(),
    })
}

fn wire_headers(headers: &Headers) -> Result<HttpHeaders, WireFailure> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| WireFailure::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| WireFailure::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}
