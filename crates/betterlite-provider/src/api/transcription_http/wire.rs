use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::{MultipartPart, encode};
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::client::SharedSession;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;
use uuid::Uuid;

use super::config::{ClientRequest, ClientSource, Headers, TranscriptionHttpConfig};
use super::error::TranscriptionHttpError;
use super::{Prepared, TranscriptionDeps};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

pub struct TranscriptionRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: TranscriptionBody,
    pub timeout: Option<Duration>,
}

// A raw body keeps its headers as they are; OutboundRequest would insert a content type httpx never sends
pub enum TranscriptionBody {
    Encoded(RequestBody),
    Raw(Vec<u8>),
}

pub trait TranscriptionTransport: Send + Sync {
    fn send(
        &self,
        request: TranscriptionRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub(super) enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    pub(super) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow TranscriptionDeps<'deps, C, S>,
    mode: ClientKind,
    litellm_params: &Object,
    shared_session: Option<&SharedSession>,
) -> Result<Selected<'borrow, S::Client>, TranscriptionHttpError>
where
    'deps: 'borrow,
    C: TranscriptionHttpConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    // _get_httpx_client takes no params, while get_async_httpx_client reads ssl_verify and the shared session
    let request = match mode {
        ClientKind::Sync => ClientRequest {
            ssl_verify: None,
            shared_session: None,
        },
        ClientKind::Asynchronous => ClientRequest {
            ssl_verify: ssl_verify(litellm_params),
            shared_session,
        },
    };
    Ok(Selected::Created(deps.source.client(request)?))
}

fn ssl_verify(litellm_params: &Object) -> Option<bool> {
    match litellm_params.get("ssl_verify") {
        Some(serde_json::Value::Bool(value)) => Some(*value),
        Some(_) | None => None,
    }
}

pub(super) fn wire_request(
    prepared: Prepared,
    timeout: Option<Duration>,
) -> Result<TranscriptionRequest, TranscriptionHttpError> {
    let Prepared { url, headers, data } = prepared;
    Ok(TranscriptionRequest {
        url,
        headers: validated_headers(&headers)?,
        body: wire_body(&headers, &data)?,
        timeout,
    })
}

fn validated_headers(headers: &Headers) -> Result<HeaderMap, TranscriptionHttpError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            TranscriptionHttpError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| TranscriptionHttpError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// httpx's encode_request: non-Mapping data is raw content, truthy files is multipart, truthy data is a form, json is JSON
fn wire_body(
    headers: &Headers,
    data: &AudioTranscriptionRequestData,
) -> Result<TranscriptionBody, TranscriptionHttpError> {
    match (&data.data, data.files.as_ref()) {
        (AudioTranscriptionData::Bytes(_), Some(files)) if !files.is_empty() => {
            Err(TranscriptionHttpError::BytesWithFiles)
        }
        (AudioTranscriptionData::Bytes(bytes), _) => Ok(TranscriptionBody::Raw(bytes.clone())),
        (AudioTranscriptionData::Object(fields), None) => {
            let body = serde_json::to_vec(fields).map_err(TranscriptionHttpError::Encode)?;
            Ok(TranscriptionBody::Encoded(RequestBody::Json(body)))
        }
        (AudioTranscriptionData::Object(fields), Some(files)) if !files.is_empty() => {
            multipart(headers, fields, files)
        }
        (AudioTranscriptionData::Object(fields), Some(_)) if fields.is_empty() => {
            Ok(TranscriptionBody::Encoded(RequestBody::Empty))
        }
        (AudioTranscriptionData::Object(fields), Some(_)) => form(fields),
    }
}

fn multipart(
    headers: &Headers,
    fields: &Object,
    files: &AudioTranscriptionFiles,
) -> Result<TranscriptionBody, TranscriptionHttpError> {
    let parts = multipart_parts(fields, files)?;
    // httpx keeps an explicit Content-Type and takes its boundary from it, generating a random one otherwise
    let Some(declared) = content_type(headers) else {
        return Ok(TranscriptionBody::Encoded(RequestBody::Multipart {
            boundary: Uuid::new_v4().simple().to_string(),
            parts,
        }));
    };
    let boundary =
        boundary_from_content_type(declared).unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let body = encode(&boundary, &parts).map_err(TranscriptionHttpError::Multipart)?;
    Ok(TranscriptionBody::Raw(body))
}

fn content_type(headers: &Headers) -> Option<&str> {
    headers
        .iter()
        .rev()
        .find(|(name, _value)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_name, value)| value.as_str())
}

fn boundary_from_content_type(declared: &str) -> Option<String> {
    if !declared.starts_with("multipart/form-data") {
        return None;
    }
    for section in declared.split(';') {
        let trimmed = section.trim();
        let Some(prefix) = trimmed.get(.."boundary=".len()) else {
            continue;
        };
        if !prefix.eq_ignore_ascii_case("boundary=") {
            continue;
        }
        let value = trimmed.get("boundary=".len()..).unwrap_or_default();
        return Some(value.trim_matches('"').to_owned());
    }
    None
}

fn multipart_parts(
    fields: &Object,
    files: &AudioTranscriptionFiles,
) -> Result<Vec<MultipartPart>, TranscriptionHttpError> {
    let mut parts: Vec<MultipartPart> = form_pairs(fields)?
        .into_iter()
        .map(|(name, text)| MultipartPart {
            name,
            file_name: None,
            content_type: None,
            data: text.into_bytes(),
        })
        .collect();
    for (name, value) in files {
        parts.push(file_part(name, value));
    }
    Ok(parts)
}

// httpx names a plain value's file upload and guesses its type from that name
fn file_part(name: &str, value: &AudioTranscriptionFileValue) -> MultipartPart {
    let data = match value {
        AudioTranscriptionFileValue::Text(text) => text.as_bytes().to_vec(),
        AudioTranscriptionFileValue::Bytes(bytes) => bytes.clone(),
    };
    MultipartPart {
        name: name.to_owned(),
        file_name: Some(String::from("upload")),
        content_type: Some(String::from("application/octet-stream")),
        data,
    }
}

fn form(fields: &Object) -> Result<TranscriptionBody, TranscriptionHttpError> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, text) in form_pairs(fields)? {
        serializer.append_pair(&name, &text);
    }
    let encoded = serializer.finish();
    Ok(TranscriptionBody::Encoded(RequestBody::Bytes {
        content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
        data: encoded.replace('*', "%2A").replace("%7E", "~").into_bytes(),
    }))
}

// A list repeats its key, as httpx's _iter_fields does for both forms
fn form_pairs(fields: &Object) -> Result<Vec<(String, String)>, TranscriptionHttpError> {
    let mut pairs = Vec::new();
    for (name, value) in fields {
        let converted: Json =
            serde_json::from_value(value.clone()).map_err(TranscriptionHttpError::Encode)?;
        match converted {
            Json::List(items) => {
                for item in &items {
                    pairs.push((name.clone(), field_text(name, item)?));
                }
            }
            item => pairs.push((name.clone(), field_text(name, &item)?)),
        }
    }
    Ok(pairs)
}

fn field_text(name: &str, value: &Json) -> Result<String, TranscriptionHttpError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Bool(flag) => Ok(flag.to_string()),
        Json::Null => Ok(String::new()),
        Json::List(_) | Json::Object(_) => Err(TranscriptionHttpError::FormField {
            name: name.to_owned(),
        }),
    }
}

mod transport;
