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
use crate::api::file_http_upload::error::{UploadError, failure_facts, status_failure};
use crate::api::file_http_upload::media::BlockIter;
use crate::api::file_http_upload::plan::{MediaPlan, PresignedPlan, TwoStepPlan, encoded_body};

use super::wire::{headers_from, request_url};
use super::{BlockStream, ClientSource, Mode, UploadBody, UploadRequest, UploadTransport};

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
    pub boundaries: &'ctx (dyn Fn() -> String + Send + Sync),
    pub timeout: Option<Duration>,
    pub limits: ApiLimits,
}

impl<C, S> Exchange<'_, '_, C, S>
where
    C: FileUploadConfig,
    S: ClientSource,
{
    // The two-step branch is the one LiteLLM wraps in try/except, so its failures become provider errors
    pub async fn two_step(&mut self, plan: &TwoStepPlan) -> Result<Reply, UploadError> {
        match self.run_two_step(plan).await {
            Ok(reply) => Ok(reply),
            Err(error) => Err(self.converted_failure(error)),
        }
    }

    // The async handler logs one line before converting; a provider error already carries its status
    fn converted_failure(&self, error: UploadError) -> UploadError {
        if self.mode == Mode::Asynchronous {
            self.emit_error(&error);
        }
        match error {
            UploadError::Provider(_) => error,
            other => UploadError::provider(&failure_facts(None, other.to_string())),
        }
    }

    async fn run_two_step(&mut self, plan: &TwoStepPlan) -> Result<Reply, UploadError> {
        let headers = self.merged_headers(&plan.initial_headers)?;
        let body = serde_json::to_vec(&plan.initial_data).map_err(UploadError::Encode)?;
        let initial = self
            .reply(
                Method::POST,
                self.api_base,
                headers,
                UploadBody::Bytes(body),
            )
            .await?;
        let (located_url, data) = located_upload(&initial.response, plan, self.limits)?;
        let Some(url) = located_url else {
            return Err(UploadError::provider(&failure_facts(
                Some(String::from(
                    "Failed to get upload URL from initial request",
                )),
                String::new(),
            )));
        };
        let method = upload_method(plan.upload_method.as_deref().unwrap_or("POST"))?;
        let upload_headers = headers_from(
            plan.upload_headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )?;
        let upload_body = encoded_body(&plan.upload_data).map_err(UploadError::Encode)?;
        let reply = self
            .reply(method, &url, upload_headers, UploadBody::Bytes(upload_body))
            .await?;
        if let Some(fields) = data {
            self.params
                .insert(String::from("initial_file_response"), Json::Object(fields));
        }
        Ok(reply)
    }

    // A presigned request is sent as the transform built it, and its failures are not converted
    pub async fn presigned(&self, plan: &PresignedPlan) -> Result<Reply, UploadError> {
        let method = upload_method(&plan.method)?;
        let headers = headers_from(
            plan.headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )?;
        let body = encoded_body(&plan.data).map_err(UploadError::Encode)?;
        self.reply(method, &plan.url, headers, UploadBody::Bytes(body))
            .await
    }

    // The media branch streams one POST and checks 200/201 itself, both inside LiteLLM's try/except
    pub async fn media(&self, plan: MediaPlan) -> Result<Reply, UploadError> {
        let result = self.run_media(plan).await;
        if let Err(error) = &result {
            self.emit_error(error);
        }
        result
    }

    async fn run_media(&self, plan: MediaPlan) -> Result<Reply, UploadError> {
        match self.media_reply(plan).await {
            Ok(reply) => match reply.response.status {
                200 | 201 => Ok(reply),
                202..=299 => Err(UploadError::provider(&failure_facts(
                    Some(format!(
                        "media upload: unexpected status {}",
                        reply.response.status
                    )),
                    String::new(),
                ))),
                _ => Err(UploadError::provider(&status_failure(
                    reply.response.status,
                    &reply.response.body,
                    &reply.response.headers,
                ))),
            },
            Err(error) => Err(UploadError::provider(&failure_facts(
                None,
                error.to_string(),
            ))),
        }
    }

    async fn media_reply(&self, plan: MediaPlan) -> Result<Reply, UploadError> {
        let headers = self.media_headers(plan.content_type.as_deref())?;
        let body = UploadBody::Blocks(self.block_stream(plan.body_stream));
        self.reply(Method::POST, self.api_base, headers, body).await
    }

    // Traditional uploads pick PUT only when the provider config asks for it, and fail as they are
    pub async fn traditional(&self, body: Vec<u8>) -> Result<Reply, UploadError> {
        let headers = self.base_headers()?;
        let method = match self.config.file_upload_http_method() {
            Method::PUT => Method::PUT,
            _ => Method::POST,
        };
        self.reply(method, self.api_base, headers, UploadBody::Bytes(body))
            .await
    }

    pub async fn multipart(&self, parts: Vec<MultipartPart>) -> Result<Reply, UploadError> {
        let headers = self.base_headers()?;
        let body = UploadBody::Multipart {
            boundary: (self.boundaries)(),
            parts,
        };
        self.reply(Method::POST, self.api_base, headers, body).await
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
        let request = UploadRequest {
            method,
            url: request_url(url)?,
            headers,
            body,
            timeout: self.timeout,
        };
        Ok(Reply {
            url: request.url.to_string(),
            response: self.client.send(request).await?,
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
        let media_type = content_type
            .filter(|value| !value.is_empty())
            .unwrap_or("application/octet-stream");
        let value =
            HeaderValue::from_str(media_type).map_err(|source| UploadError::HeaderValue {
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
        Box::pin(stream::unfold(receiver, |mut channel| async move {
            channel.recv().await.map(|block| (block, channel))
        }))
    }
}

// the headers location is the default and reads no key; only a body location parses the answer
fn located_upload(
    response: &UpstreamResponse,
    plan: &TwoStepPlan,
    limits: ApiLimits,
) -> Result<(Option<String>, Option<IndexMap<String, Json>>), UploadError> {
    match plan.url_location {
        Some(UploadUrlLocation::Body) => {
            let located = decode_file_upload_location(
                &response.body,
                UploadUrlLocation::Body,
                plan.url_key.as_deref().unwrap_or("upload_url"),
                limits,
            )?;
            Ok((located.url, located.data))
        }
        Some(UploadUrlLocation::Headers) | None => {
            Ok((upload_url_from_headers(&response.headers), None))
        }
    }
}

// The sync handler decodes a bytes body before sending it, so invalid UTF-8 fails before HTTP
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
