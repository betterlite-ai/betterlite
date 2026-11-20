use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::openai_base_config::{Headers, Object};
use bytes::Bytes;
use http::Method;
use http::header::{COOKIE, HeaderMap, HeaderName, HeaderValue};
use url::Url;
use url::form_urlencoded;
use uuid::Uuid;

use super::PassthroughUpload;
use super::config::ConfigError;

const OCTET_STREAM: &str = "application/octet-stream";
const FORM_URLENCODED: &str = "application/x-www-form-urlencoded";

pub(crate) struct WireRequest<'call> {
    pub method: &'call str,
    pub url: &'call Url,
    pub headers: &'call Headers,
    pub content: Option<&'call Bytes>,
    pub signed: Option<&'call [u8]>,
    pub data: Option<&'call Object>,
    pub files: Option<&'call [PassthroughUpload]>,
    pub json: Option<&'call JsonValue>,
    pub params: Option<&'call [(String, String)]>,
    pub cookies: Option<&'call [(String, String)]>,
    pub deadline: Duration,
}

// client.client.build_request: signed bytes win over content, and both silence data, files and json
pub(crate) fn build(request: &WireRequest<'_>) -> Result<OutboundRequest, ConfigError> {
    let method =
        Method::from_bytes(request.method.as_bytes()).map_err(|error| ConfigError::ValueError {
            message: format!("method {} is not valid: {error}", request.method),
        })?;
    let mut headers = header_map(request.headers)?;
    if let Some(cookie) = cookie_header(request.cookies)? {
        headers.insert(COOKIE, cookie);
    }
    Ok(OutboundRequest {
        method,
        url: request.url.clone(),
        query: request
            .params
            .map(<[(String, String)]>::to_vec)
            .unwrap_or_default(),
        headers,
        body: body(request)?,
        deadline: request.deadline,
    })
}

fn body(request: &WireRequest<'_>) -> Result<RequestBody, ConfigError> {
    if let Some(signed) = request.signed {
        return Ok(RequestBody::Json(signed.to_vec()));
    }
    if let Some(content) = request.content {
        return Ok(RequestBody::Bytes {
            content_type: HeaderValue::from_static(OCTET_STREAM),
            data: content.to_vec(),
        });
    }
    if let Some(files) = request.files {
        return multipart(files, request.data);
    }
    if let Some(data) = request.data {
        return urlencoded(data);
    }
    request.json.map_or_else(
        || Ok(RequestBody::Empty),
        |json| {
            serde_json::to_vec(json)
                .map(RequestBody::Json)
                .map_err(|error| ConfigError::ValueError {
                    message: error.to_string(),
                })
        },
    )
}

fn header_map(headers: &Headers) -> Result<HeaderMap, ConfigError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|error| ConfigError::ValueError {
                message: format!("header name {name} is not valid: {error}"),
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|error| ConfigError::ValueError {
                message: format!("header {name} is not valid: {error}"),
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// httpx's Cookies jar sets the Cookie header itself, so the jar replaces a caller header of the same name
fn cookie_header(cookies: Option<&[(String, String)]>) -> Result<Option<HeaderValue>, ConfigError> {
    let Some(pairs) = cookies.filter(|jar| !jar.is_empty()) else {
        return Ok(None);
    };
    let joined = pairs
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    HeaderValue::from_str(&joined)
        .map(Some)
        .map_err(|error| ConfigError::ValueError {
            message: format!("cookies are not a valid header value: {error}"),
        })
}

// encode_multipart_data: data fields keep their form order, then every upload becomes a file part
fn multipart(
    files: &[PassthroughUpload],
    data: Option<&Object>,
) -> Result<RequestBody, ConfigError> {
    let mut parts = Vec::new();
    if let Some(fields) = data {
        for (name, value) in fields {
            parts.push(MultipartPart {
                name: name.clone(),
                file_name: None,
                content_type: None,
                data: primitive_text(name, value)?.into_bytes(),
            });
        }
    }
    for upload in files {
        parts.push(MultipartPart {
            name: upload.field.clone(),
            file_name: Some(upload.filename.clone()),
            content_type: upload.content_type.clone(),
            data: upload.bytes.to_vec(),
        });
    }
    Ok(RequestBody::Multipart {
        boundary: Uuid::new_v4().simple().to_string(),
        parts,
    })
}

fn urlencoded(data: &Object) -> Result<RequestBody, ConfigError> {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in data {
        if let JsonValue::Array(items) = value {
            for item in items {
                serializer.append_pair(name, &primitive_text(name, item)?);
            }
        } else {
            serializer.append_pair(name, &primitive_text(name, value)?);
        }
    }
    Ok(RequestBody::Bytes {
        content_type: HeaderValue::from_static(FORM_URLENCODED),
        data: serializer.finish().into_bytes(),
    })
}

// httpx's primitive_value_to_str, which refuses a nested list or a mapping inside form data
fn primitive_text(name: &str, value: &JsonValue) -> Result<String, ConfigError> {
    match value {
        JsonValue::Null => Ok(String::new()),
        JsonValue::Bool(flag) => Ok(String::from(if *flag { "true" } else { "false" })),
        JsonValue::Number(number) => Ok(number.to_string()),
        JsonValue::String(text) => Ok(text.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => Err(ConfigError::ValueError {
            message: format!("form field {name} must be a string, a number or a boolean"),
        }),
    }
}

// headers and reply pairs share one text form: a value that is not printable text keeps its bytes as latin-1
pub(crate) fn header_text(value: &HeaderValue) -> String {
    value.to_str().map_or_else(
        |_| {
            value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect()
        },
        str::to_owned,
    )
}
