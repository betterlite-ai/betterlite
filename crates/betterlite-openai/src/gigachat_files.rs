use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::alphabet;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderValue, Method, header::CONTENT_TYPE};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

use crate::gigachat_chat::config::{GigachatBearer, GigachatImageUploader};
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::{IdSource, get_api_base};

mod tests;

pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

const BASE64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const DATA_URL_PREFIX: &str = "data:";
const DEFAULT_CONTENT_TYPE: &str = "image/jpeg";
const DEFAULT_EXTENSION: &str = "jpg";
const FILE_FIELD: &str = "file";
const FILES_SUFFIX: &str = "/files";
const PURPOSE_FIELD: &str = "purpose";
const PURPOSE_VALUE: &str = "general";

// Python's b64decode keeps nonzero trailing bits and requires canonical padding, which we add
const DATA_URL_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

// download must skip certificate checks and follow redirects like LiteLLM's client; upload must verify certificates
pub trait GigachatFileTransport: Send + Sync {
    fn download(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;

    fn upload(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub struct GigachatFilesConfig {
    pub bearer: Arc<dyn GigachatBearer>,
    pub ids: Arc<dyn IdSource>,
    pub credentials: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_base_env: Option<String>,
}

pub struct GigachatFileUploader<T> {
    transport: T,
    bearer: Arc<dyn GigachatBearer>,
    ids: Arc<dyn IdSource>,
    credentials: Option<SecretString>,
    api_base: Option<String>,
    api_base_env: Option<String>,
    capacity: usize,
    cache: Mutex<IndexMap<String, String>>,
}

impl<T: GigachatFileTransport> GigachatFileUploader<T> {
    #[must_use]
    pub fn new(transport: T, config: GigachatFilesConfig, capacity: usize) -> Self {
        Self {
            transport,
            bearer: config.bearer,
            ids: config.ids,
            credentials: config.credentials,
            api_base: config.api_base,
            api_base_env: config.api_base_env,
            capacity,
            cache: Mutex::new(IndexMap::new()),
        }
    }

    async fn try_upload(&self, image_url: &str) -> Result<String, UploadError> {
        let key = hex::encode(Sha256::digest(image_url.as_bytes()));
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(id) = cached {
            return Ok(id);
        }
        let image = self.image(image_url).await?;
        let filename = format!("{}.{}", self.ids.new_uuid(), image.extension);
        let credentials = self
            .credentials
            .as_ref()
            .filter(|value| !value.expose_secret().is_empty());
        let token = self
            .bearer
            .access_token(credentials)
            .await
            .map_err(UploadError::Token)?;
        let reply = self.send_upload(&token, filename, image).await?;
        if reply.status >= 400 {
            return Err(UploadError::UploadStatus(reply.status));
        }
        let UploadReply { id } = serde_json::from_slice::<UploadReply>(&reply.body)?;
        let Some(id) = id.filter(|value| !value.is_empty()) else {
            return Err(UploadError::MissingId);
        };
        if self.capacity > 0 {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if cache.len() >= self.capacity {
                cache.shift_remove_index(0);
            }
            cache.insert(key, id.clone());
        }
        Ok(id)
    }

    async fn image(&self, image_url: &str) -> Result<Image, UploadError> {
        if let Some(parsed) = parse_data_url(image_url) {
            let data = decode_payload(parsed.payload).ok_or(UploadError::Base64)?;
            return Ok(Image {
                extension: extension_of(parsed.content_type),
                content_type: parsed.content_type.to_owned(),
                data,
            });
        }
        let url = Url::parse(image_url).map_err(|source| UploadError::Url {
            what: "image",
            source,
        })?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: DOWNLOAD_TIMEOUT,
        };
        let reply =
            self.transport
                .download(request)
                .await
                .map_err(|source| UploadError::Transport {
                    step: "download",
                    source,
                })?;
        if reply.status >= 400 {
            return Err(UploadError::DownloadStatus(reply.status));
        }
        let content_type = reply
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map_or_else(|| String::from(DEFAULT_CONTENT_TYPE), str::to_owned);
        Ok(Image {
            extension: extension_of(&content_type),
            content_type,
            data: reply.body,
        })
    }

    async fn send_upload(
        &self,
        token: &SecretString,
        filename: String,
        image: Image,
    ) -> Result<UpstreamResponse, UploadError> {
        let base = get_api_base(self.api_base.as_deref(), self.api_base_env.as_deref());
        let url =
            Url::parse(&format!("{base}{FILES_SUFFIX}")).map_err(|source| UploadError::Url {
                what: "upload",
                source,
            })?;
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))?,
        );
        let request = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Multipart {
                boundary: self.ids.new_uuid().simple().to_string(),
                parts: vec![
                    MultipartPart {
                        name: String::from(PURPOSE_FIELD),
                        file_name: None,
                        content_type: None,
                        data: PURPOSE_VALUE.as_bytes().to_vec(),
                    },
                    MultipartPart {
                        name: String::from(FILE_FIELD),
                        file_name: Some(filename),
                        content_type: Some(image.content_type),
                        data: image.data,
                    },
                ],
            },
            deadline: UPLOAD_TIMEOUT,
        };
        self.transport
            .upload(request)
            .await
            .map_err(|source| UploadError::Transport {
                step: "upload",
                source,
            })
    }
}

#[async_trait]
impl<T: GigachatFileTransport> GigachatImageUploader for GigachatFileUploader<T> {
    async fn upload(&self, image_url: &str) -> Option<String> {
        match self.try_upload(image_url).await {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::error!(error = %error, "Error uploading file to GigaChat");
                None
            }
        }
    }
}

#[derive(Debug, Error)]
enum UploadError {
    #[error("the data url payload is not base64")]
    Base64,
    #[error("the image download answered with status {0}")]
    DownloadStatus(u16),
    #[error("the upload reply carries no file id")]
    MissingId,
    #[error("the access token could not be fetched: {0}")]
    Token(#[source] GigaChatError),
    #[error("the {step} request failed: {source}")]
    Transport {
        step: &'static str,
        #[source]
        source: TransportError,
    },
    #[error("the upload answered with status {0}")]
    UploadStatus(u16),
    #[error("the {what} url is not valid: {source}")]
    Url {
        what: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("the upload reply cannot be read: {source}")]
    Reply {
        #[from]
        source: serde_json::Error,
    },
    #[error("the authorization header is not valid: {source}")]
    Header {
        #[from]
        source: http::header::InvalidHeaderValue,
    },
}

#[derive(Debug, Deserialize)]
struct UploadReply {
    #[serde(default)]
    id: Option<String>,
}

struct Image {
    content_type: String,
    extension: String,
    data: Vec<u8>,
}

struct DataUrl<'text> {
    content_type: &'text str,
    payload: &'text str,
}

fn extension_of(content_type: &str) -> String {
    let subtype = content_type.rsplit('/').next().unwrap_or_default();
    let extension = subtype.split(';').next().unwrap_or_default();
    if extension.is_empty() {
        String::from(DEFAULT_EXTENSION)
    } else {
        extension.to_owned()
    }
}

fn parse_data_url(text: &str) -> Option<DataUrl<'_>> {
    let rest = text.strip_prefix(DATA_URL_PREFIX)?;
    let (content_type, tail) = rest.split_once(';')?;
    if content_type.is_empty() {
        return None;
    }
    let payload = tail.strip_prefix("base64,")?.split('\n').next()?;
    if payload.is_empty() {
        return None;
    }
    Some(DataUrl {
        content_type,
        payload,
    })
}

// Python's b64decode discards characters outside the alphabet and accepts extra '=' padding
fn decode_payload(payload: &str) -> Option<Vec<u8>> {
    if !payload.is_ascii() {
        return None;
    }
    let filtered: String = payload
        .bytes()
        .filter(|byte| BASE64_ALPHABET.contains(byte) || *byte == b'=')
        .map(char::from)
        .collect();
    let (data, padded) = match filtered.split_once('=') {
        Some((data, rest)) if rest.bytes().all(|byte| byte == b'=') => (data, true),
        Some(_) => return None,
        None => (filtered.as_str(), false),
    };
    let remainder = data.len() % 4;
    if remainder == 1 || (remainder != 0 && !padded) {
        return None;
    }
    let mut canonical = String::from(data);
    canonical.push_str(&"=".repeat((4 - remainder) % 4));
    DATA_URL_BASE64.decode(canonical).ok()
}
