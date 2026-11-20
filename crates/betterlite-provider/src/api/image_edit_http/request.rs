use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageParams,
};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use super::error::RequestError;
use super::service::{ClientKind, LogContext};

// The typed litellm_params values the handler reads before it selects a client
pub(super) struct Fields {
    pub(super) api_key: Option<SecretString>,
    pub(super) api_base: Option<String>,
    pub(super) ssl_verify: Option<bool>,
}

// The pre-call the handler logs before the send: the prompt, the resolved URL, the headers, and the body each mode logs
#[derive(Debug, Clone, Copy)]
pub(super) struct Logged<'logged> {
    pub(super) prompt: Option<&'logged str>,
    pub(super) api_base: &'logged str,
    pub(super) headers: &'logged ImageHeaders,
    pub(super) data: &'logged ImageParams,
    pub(super) files: &'logged ImageFiles,
}

pub(super) fn fields(params: &ImageParams) -> Result<Fields, RequestError> {
    Ok(Fields {
        api_key: text(params, "api_key")?.map(SecretString::from),
        api_base: text(params, "api_base")?,
        ssl_verify: flag(params)?,
    })
}

// The header map the environment hook is asked to complete: only the caller's own extra_headers entries
pub(super) fn caller_headers(optional: &ImageParams) -> Result<ImageHeaders, RequestError> {
    match optional.get("extra_headers") {
        None | Some(Json::Null) => Ok(ImageHeaders::new()),
        Some(Json::Object(entries)) => entries
            .iter()
            .map(|(name, value)| match value {
                Json::Text(text) => Ok((name.clone(), text.clone())),
                Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(RequestError::FieldType {
                        field: String::from("optional_params.extra_headers"),
                    })
                }
            })
            .collect(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(RequestError::FieldType {
                field: String::from("optional_params.extra_headers"),
            })
        }
    }
}

pub(super) fn merge(mut validated: ImageHeaders, explicit: Option<&ImageHeaders>) -> ImageHeaders {
    if let Some(extra) = explicit.filter(|extra| !extra.is_empty()) {
        for (name, value) in extra {
            validated.insert(name.clone(), value.clone());
        }
    }
    validated
}

pub(super) fn endpoint(resolved: &str) -> Result<Url, RequestError> {
    let url = resolved.to_owned();
    Url::parse(resolved).map_err(|source| RequestError::Url { url, source })
}

pub(super) fn wire_headers(headers: &ImageHeaders) -> Result<HeaderMap, RequestError> {
    headers
        .iter()
        .map(|(name, value)| {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
                RequestError::HeaderName {
                    name: name.clone(),
                    source,
                }
            })?;
            let header_value =
                HeaderValue::from_str(value).map_err(|source| RequestError::HeaderValue {
                    name: name.clone(),
                    source,
                })?;
            Ok((header_name, header_value))
        })
        .collect()
}

// The finalized data as form fields, then the transform's files; a list repeats its field, and a nested value has no form spelling
pub(super) fn multipart_parts(
    data: &ImageParams,
    files: &ImageFiles,
) -> Result<Vec<MultipartPart>, RequestError> {
    let mut parts = Vec::new();
    for (name, value) in data {
        match value {
            Json::List(items) => {
                for item in items {
                    parts.push(field_part(name, item)?);
                }
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                parts.push(field_part(name, value)?);
            }
        }
    }
    for (name, file) in files {
        parts.push(file_part(name, file));
    }
    Ok(parts)
}

fn field_part(name: &str, value: &Json) -> Result<MultipartPart, RequestError> {
    let text = match value {
        Json::Null => String::new(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => {
            return Err(RequestError::FormValue {
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

// A file part carries its own name, its file name when it has one, and the given content type or the one its file name implies
fn file_part(name: &str, file: &ImageFile) -> MultipartPart {
    let file_name = file
        .filename
        .clone()
        .filter(|filename| !filename.is_empty());
    let content_type = file.content_type.clone().or_else(|| {
        file_name.as_deref().map(|filename| {
            mime_guess::from_path(filename)
                .first_or_octet_stream()
                .to_string()
        })
    });
    MultipartPart {
        name: name.to_owned(),
        file_name,
        content_type,
        data: file.content.clone(),
    }
}

pub(super) fn log_pre_call(
    logged: Logged<'_>,
    mode: ClientKind,
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    let mut headers = IndexMap::new();
    for (name, value) in logged.headers {
        headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let complete = match mode {
        ClientKind::Sync => RawBody::Object(file_mapping(logged.files)),
        ClientKind::Asynchronous => RawBody::Object(logged.data.clone()),
    };
    let record = PreCall {
        input: logged
            .prompt
            .map_or(Json::Null, |prompt| Json::Text(prompt.to_owned())),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(logged.api_base.to_owned()),
            headers: Some(headers),
            complete_input_dict: Some(complete),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)
}

// The files mapping LiteLLM's sync handler logs, as JSON: each entry keeps the tuple's filename, content and content type
fn file_mapping(files: &ImageFiles) -> IndexMap<String, Json> {
    let mut mapping = IndexMap::new();
    for (name, file) in files {
        let mut record = IndexMap::new();
        let content = std::str::from_utf8(&file.content);
        record.insert(
            String::from("filename"),
            text_or_null(file.filename.as_deref()),
        );
        record.insert(String::from("content"), text_or_null(content));
        record.insert(
            String::from("content_type"),
            text_or_null(file.content_type.as_deref()),
        );
        mapping.insert(name.clone(), Json::Object(record));
    }
    mapping
}

fn text_or_null(value: Option<&str>) -> Json {
    value.map_or(Json::Null, |text| Json::Text(text.to_owned()))
}

fn text(params: &ImageParams, name: &str) -> Result<Option<String>, RequestError> {
    match params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RequestError::FieldType {
                field: format!("litellm_params.{name}"),
            })
        }
    }
}

fn flag(params: &ImageParams) -> Result<Option<bool>, RequestError> {
    match params.get("ssl_verify") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(verify)) => Ok(Some(*verify)),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RequestError::FieldType {
                field: String::from("litellm_params.ssl_verify"),
            })
        }
    }
}
