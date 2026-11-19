use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_pre_post::env::{LogLevel, PrePostEnv};
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{ByteStream, UploadUrlLocation};
use futures_util::stream;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio::sync::mpsc;
use tokio_util::task::TaskTracker;

use crate::api::api_boundaries::ApiLimits;
use crate::api::file_http_upload::config::{FileParams, FileUploadConfig, Headers};
use crate::api::file_http_upload::decoder::{decode_file_upload_location, upload_url_from_headers};
use crate::api::file_http_upload::error::{
    UploadError, converted, display_failure, status_failure, value_failure,
};
use crate::api::file_http_upload::media::BlockIter;
use crate::api::file_http_upload::plan::{
    MediaPlan, PresignedPlan, TwoStepPlan, encoded_body, initial_body,
};

use super::{
    BlockStream, ClientSource, Mode, Reply, UploadBody, UploadRequest, UploadTransport,
    headers_from, request_url,
};

const MISSING_UPLOAD_URL: &str = "Failed to get upload URL from initial request";
const DEFAULT_UPLOAD_METHOD: &str = "POST";
const DEFAULT_UPLOAD_URL_KEY: &str = "upload_url";
const DEFAULT_MEDIA_CONTENT_TYPE: &str = "application/octet-stream";

// The answer transform_create_file_response reads, with the URL the last request was sent to
pub struct Reply {
    pub response: UpstreamResponse,
    pub url: String,
}

pub struct Exchange<'ctx, 'env, C: FileUploadConfig, S: ClientSource> {
    pub config: &'ctx C,
    pub client: &'ctx S::Client,
    pub api_base: &'ctx str,
    pub headers: &'ctx Headers,
    pub params: &'ctx mut FileParams,
    pub env: &'ctx PrePostEnv<'env>,
    pub mode: Mode,
    pub tasks: &'ctx TaskTracker,
    pub boundaries: &'ctx dyn Fn() -> String,
    pub timeout: Option<Duration>,
    pub limits: ApiLimits,
}

impl<C, S> Exchange<'_, '_, C, S>
where
    C: FileUploadConfig,
    S: ClientSource,
{
    // The two-step branch is the one LiteLLM wraps in try/except: its failures become provider errors,
    // and the async handler logs one line first
    pub async fn two_step(&mut self, plan: &TwoStepPlan) -> Result<Reply, UploadError> {
        match self.run_two_step(plan).await {
            Ok(reply) => Ok(reply),
            Err(error) => {
                if self.mode == Mode::Asynchronous {
                    self.emit_error(&error);
                }
                match error {
                    UploadError::Provider(_) => Err(error),
                    other => Err(converted(&display_failure(other.to_string()))),
                }
            }
        }
    }

    async fn run_two_step(&mut self, plan: &TwoStepPlan) -> Result<Reply, UploadError> {
        let headers = self.merged_headers(&plan.initial_headers)?;
        let body = initial_body(&plan.initial_data).map_err(UploadError::Encode)?;
        let initial = self
            .reply(Method::POST, self.api_base, headers, body)
            .await?;
        let (url, data) = match plan.url_location {
            Some(UploadUrlLocation::Body) => {
                let located = decode_file_upload_location(
                    &initial.response.body,
                    UploadUrlLocation::Body,
                    plan.url_key.as_deref().unwrap_or(DEFAULT_UPLOAD_URL_KEY),
                    self.limits,
                )?;
                (located.url, located.data)
            }
            // the location defaults to headers, and no key is read there
            Some(UploadUrlLocation::Headers) | None => {
                (upload_url_from_headers(&initial.response.headers), None)
            }
        };
        let Some(url) = url else {
            return Err(converted(&value_failure(String::from(MISSING_UPLOAD_URL))));
        };
        let method = upload_method(
            plan.upload_method
                .as_deref()
                .unwrap_or(DEFAULT_UPLOAD_METHOD),
        )?;
        let headers = headers_from(
            plan.upload_headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )?;
        let body = encoded_body(&plan.upload_data).map_err(UploadError::Encode)?;
        let reply = self.reply(method, &url, headers, body).await?;
        if let Some(fields) = data {
            self.params
                .insert(String::from("initial_file_response"), Json::Object(fields));
        }
        Ok(reply)
    }

    // A presigned request is sent exactly as the transform built it, and its failures are not converted
    pub async fn presigned(&mut self, plan: &PresignedPlan) -> Result<Reply, UploadError> {
        let method = upload_method(&plan.method)?;
        let headers = headers_from(
            plan.headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )?;
        let body = encoded_body(&plan.data).map_err(UploadError::Encode)?;
        self.reply(method, &plan.url, headers, body).await
    }

    // The media branch streams one POST and checks 200/201 itself, both inside LiteLLM's try/except
    pub async fn media(&mut self, plan: MediaPlan) -> Result<Reply, UploadError> {
        let result = self.run_media(plan).await;
        if let Err(error) = &result {
            self.emit_error(error);
        }
        result
    }

    async fn run_media(&mut self, plan: MediaPlan) -> Result<Reply, UploadError> {
        match self.media_reply(plan).await {
            Ok(reply) => match reply.response.status {
                200 | 201 => Ok(reply),
                202..=299 => Err(converted(&value_failure(format!(
                    "media upload: unexpected status {}",
                    reply.response.status
                )))),
                _ => Err(converted(&status_failure(
                    reply.response.status,
                    &reply.response.body,
                    &reply.response.headers,
                ))),
            },
            Err(error) => Err(converted(&display_failure(error.to_string()))),
        }
    }

    async fn media_reply(&mut self, plan: MediaPlan) -> Result<Reply, UploadError> {
        let headers = self.media_headers(plan.content_type.as_deref())?;
        let body = UploadBody::Blocks(self.block_stream(plan.body_stream));
        self.reply(Method::POST, self.api_base, headers, body).await
    }

    // Traditional uploads pick PUT only when the provider config asks for it, and fail as they are
    pub async fn traditional(&mut self, body: Vec<u8>) -> Result<Reply, UploadError> {
        let method = match self.config.file_upload_http_method() {
            Method::PUT => Method::PUT,
            _ => Method::POST,
        };
        let headers = self.base_headers()?;
        self.reply(method, self.api_base, headers, UploadBody::Bytes(body))
            .await
    }

    pub async fn multipart(&mut self, parts: Vec<MultipartPart>) -> Result<Reply, UploadError> {
        let headers = self.base_headers()?;
        let boundary = (self.boundaries)();
        self.reply(
            Method::POST,
            self.api_base,
            headers,
            UploadBody::Multipart { boundary, parts },
        )
        .await
    }

    pub fn emit_error(&self, error: &UploadError) {
        self.env.emit(
            LogLevel::Error,
            &format!("Error creating file: {error}"),
            IndexMap::new(),
        );
    }

    async fn reply(
        &self,
        method: Method,
        url: &str,
        headers: HeaderMap,
        body: UploadBody,
    ) -> Result<Reply, UploadError> {
        let request = self.request(method, url, headers, body)?;
        Ok(Reply {
            url: request.url.to_string(),
            response: self.client.send(request).await?,
        })
    }

    fn request(
        &self,
        method: Method,
        url: &str,
        headers: HeaderMap,
        body: UploadBody,
    ) -> Result<UploadRequest, UploadError> {
        Ok(UploadRequest {
            method,
            url: request_url(url)?,
            headers,
            body,
            timeout: self.timeout,
        })
    }

    fn base_headers(&self) -> Result<HeaderMap, UploadError> {
        headers_from(
            self.headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose_secret().as_str())),
        )
    }

    // LiteLLM sends {**headers, **initial_request["headers"]}, so the transform's own entries win
    fn merged_headers(&self, extra: &BTreeMap<String, String>) -> Result<HeaderMap, UploadError> {
        let base = self
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.expose_secret().as_str()));
        let extra = extra
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()));
        headers_from(base.chain(extra))
    }

    fn media_headers(&self, content_type: Option<&str>) -> Result<HeaderMap, UploadError> {
        let mut headers = self.base_headers()?;
        let content_type = content_type
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_MEDIA_CONTENT_TYPE);
        let value =
            HeaderValue::from_str(content_type).map_err(|source| UploadError::HeaderValue {
                name: String::from("content-type"),
                source,
            })?;
        headers.insert(CONTENT_TYPE, value);
        Ok(headers)
    }

    // The block iterator runs on a blocking thread, so no transform work touches the async executor
    fn block_stream(&self, body_stream: ByteStream) -> BlockStream {
        let (sender, receiver) = mpsc::channel::<Vec<u8>>(1);
        self.tasks.spawn_blocking(move || {
            for block in BlockIter::new(body_stream) {
                if sender.blocking_send(block).is_err() {
                    return;
                }
            }
        });
        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|block| (block, receiver))
        }))
    }
}

// The sync handler decodes a bytes body to text before sending it, so invalid UTF-8 fails before HTTP;
// the async handler hands the bytes to httpx as they are
pub fn traditional_body(mode: Mode, bytes: &[u8]) -> Result<Vec<u8>, UploadError> {
    match mode {
        Mode::Sync => {
            std::str::from_utf8(bytes).map_err(UploadError::Decode)?;
            Ok(bytes.to_vec())
        }
        Mode::Asynchronous => Ok(bytes.to_vec()),
    }
}

// getattr(client, upload_method.lower()): the method name is uppercased back to the wire token
fn upload_method(method: &str) -> Result<Method, UploadError> {
    let token = method.to_ascii_uppercase();
    Method::from_bytes(token.as_bytes()).map_err(|_source| UploadError::Method {
        method: method.to_owned(),
    })
}
