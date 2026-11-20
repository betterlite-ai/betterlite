use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_pre_post::env::{LogLevel, PrePostEnv};
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{ByteStream, UploadUrlLocation};
use futures_util::stream;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio::sync::mpsc;
use tokio_util::task::TaskTracker;

use crate::api::api_boundaries::ApiLimits;
use crate::api::file_http_upload::config::{FileParams, FileUploadConfig, Headers};
use crate::api::file_http_upload::decoder::{
    LocatedUpload, decode_file_upload_location, upload_url_from_headers,
};
use crate::api::file_http_upload::error::{
    UploadError, failure_facts, raise_for_status, status_failure,
};
use crate::api::file_http_upload::media::BlockIter;
use crate::api::file_http_upload::plan::{MediaPlan, PresignedPlan, TwoStepPlan, encoded_body};
use crate::api::vector_store_http_write::error::handle_error;

use super::wire::{default_content_type, headers_from, media_headers, request_url, upload_method};
use super::{BlockStream, Mode, UploadBody, UploadRequest, UploadTransport};

// The answer transform_create_file_response reads, with the URL the last request was sent to
pub(super) struct Reply {
    pub response: UpstreamResponse,
    pub url: String,
}

pub(super) struct Exchange<'ctx, 'env, C: FileUploadConfig, Client> {
    pub config: &'ctx C,
    pub client: &'ctx Client,
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

impl<C, Client> Exchange<'_, '_, C, Client>
where
    C: FileUploadConfig,
    Client: UploadTransport,
{
    // The two-step branch is the one LiteLLM wraps in try/except, so its failures become provider errors
    pub(super) async fn two_step(&mut self, plan: &TwoStepPlan) -> Result<Reply, UploadError> {
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
        if matches!(error, UploadError::Provider(_)) {
            return error;
        }
        let facts = failure_facts(None, error.to_string());
        UploadError::Provider(handle_error(&facts, None))
    }

    async fn run_two_step(&mut self, plan: &TwoStepPlan) -> Result<Reply, UploadError> {
        let headers = self.merged_headers(&plan.initial_headers)?;
        let body = serde_json::to_vec(&plan.initial_data).map_err(UploadError::Encode)?;
        let initial = self
            .reply(
                &Method::POST,
                self.api_base,
                headers,
                UploadBody::Bytes(body),
            )
            .await?;
        raise_for_status(self.mode, &Method::POST, &initial.response)?;
        let UploadTarget { url, data } = located_upload(&initial.response, plan, self.limits)?;
        let method = upload_method(plan.upload_method.as_deref().unwrap_or("POST"))?;
        let upload_headers = headers_from(
            plan.upload_headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )?;
        let upload_body = encoded_body(&plan.upload_data).map_err(UploadError::Encode)?;
        let reply = self
            .reply(
                &method,
                &url,
                upload_headers,
                UploadBody::Bytes(upload_body),
            )
            .await?;
        raise_for_status(self.mode, &method, &reply.response)?;
        if let Some(fields) = data {
            self.params
                .insert(String::from("initial_file_response"), Json::Object(fields));
        }
        Ok(reply)
    }

    // A presigned request is sent as the transform built it, and its failures are not converted
    pub(super) async fn presigned(&self, plan: &PresignedPlan) -> Result<Reply, UploadError> {
        let method = upload_method(&plan.method)?;
        let headers = headers_from(
            plan.headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )?;
        let body = encoded_body(&plan.data).map_err(UploadError::Encode)?;
        let reply = self
            .reply(&method, &plan.url, headers, UploadBody::Bytes(body))
            .await?;
        raise_for_status(self.mode, &method, &reply.response)?;
        Ok(reply)
    }

    // The media branch streams one POST and checks 200/201 itself, both inside LiteLLM's try/except
    pub(super) async fn media(&self, plan: MediaPlan) -> Result<Reply, UploadError> {
        let result = self.run_media(plan).await;
        if let Err(error) = &result {
            self.emit_error(error);
        }
        result
    }

    async fn run_media(&self, plan: MediaPlan) -> Result<Reply, UploadError> {
        let headers = media_headers(self.base_headers()?, plan.content_type.as_deref())?;
        let body = UploadBody::Blocks(self.block_stream(plan.body_stream));
        match self
            .reply(&Method::POST, self.api_base, headers, body)
            .await
        {
            Ok(reply) => match reply.response.status {
                200 | 201 => Ok(reply),
                202..=299 => {
                    let facts = failure_facts(
                        Some(format!(
                            "media upload: unexpected status {}",
                            reply.response.status
                        )),
                        String::new(),
                    );
                    Err(UploadError::Provider(handle_error(&facts, None)))
                }
                _ => Err(UploadError::Provider(handle_error(
                    &status_failure(
                        reply.response.status,
                        &reply.response.body,
                        &reply.response.headers,
                    ),
                    None,
                ))),
            },
            Err(error) => Err(UploadError::Provider(handle_error(
                &failure_facts(None, error.to_string()),
                None,
            ))),
        }
    }

    // A text, byte or multipart plan sends its body to api_base; only a traditional body uses PUT
    pub(super) async fn base_body(&self, body: UploadBody) -> Result<Reply, UploadError> {
        let method = match &body {
            UploadBody::Bytes(_) => match self.config.file_upload_http_method() {
                Method::PUT => Method::PUT,
                _ => Method::POST,
            },
            UploadBody::Multipart { .. } | UploadBody::Blocks(_) => Method::POST,
        };
        let reply = self
            .reply(&method, self.api_base, self.base_headers()?, body)
            .await?;
        raise_for_status(self.mode, &method, &reply.response)?;
        Ok(reply)
    }

    pub(super) async fn multipart(&self, parts: Vec<MultipartPart>) -> Result<Reply, UploadError> {
        let body = UploadBody::Multipart {
            boundary: (self.boundaries)(),
            parts,
        };
        self.base_body(body).await
    }

    fn emit_error(&self, error: &UploadError) {
        self.env.emit(
            LogLevel::Error,
            &format!("Error creating file: {error}"),
            IndexMap::new(),
        );
    }

    async fn reply(
        &self,
        method: &Method,
        url: &str,
        headers: HeaderMap,
        body: UploadBody,
    ) -> Result<Reply, UploadError> {
        let sent_headers = match &body {
            UploadBody::Bytes(_) => default_content_type(headers, self.mode),
            UploadBody::Multipart { .. } | UploadBody::Blocks(_) => headers,
        };
        let request = UploadRequest {
            method: method.clone(),
            url: request_url(url)?,
            headers: sent_headers,
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
                .map(|(name, value)| (name.as_str(), value.expose_secret())),
        )
    }

    // LiteLLM sends {**headers, **initial_request["headers"]}, so the transform's own entries win
    fn merged_headers(&self, extra: &BTreeMap<String, String>) -> Result<HeaderMap, UploadError> {
        let base = self
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.expose_secret()));
        let transform_headers = extra
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()));
        headers_from(base.chain(transform_headers))
    }

    // The block iterator runs on a blocking thread, so no transform work touches the async executor
    fn block_stream(&self, body_stream: ByteStream) -> BlockStream {
        let (sender, receiver) = mpsc::channel::<Vec<u8>>(1);
        self.tasks.spawn_blocking(move || {
            // A reader that went away closes the channel, which stops the producer at once
            BlockIter::new(body_stream)
                .try_for_each(|block| sender.blocking_send(block))
                .unwrap_or_default();
        });
        Box::pin(stream::unfold(receiver, |mut channel| async move {
            channel.recv().await.map(|block| (block, channel))
        }))
    }
}

// The resolved second-request target: the URL and the initial body the transform may read
struct UploadTarget {
    url: String,
    data: Option<IndexMap<String, Json>>,
}

// The headers location is the default and reads no key; a missing URL is the ValueError the two-step converts
fn located_upload(
    response: &UpstreamResponse,
    plan: &TwoStepPlan,
    limits: ApiLimits,
) -> Result<UploadTarget, UploadError> {
    let located = match plan.url_location {
        Some(UploadUrlLocation::Body) => decode_file_upload_location(
            &response.body,
            UploadUrlLocation::Body,
            plan.url_key.as_deref().unwrap_or("upload_url"),
            limits,
        )?,
        Some(UploadUrlLocation::Headers) | None => LocatedUpload {
            url: upload_url_from_headers(&response.headers),
            data: None,
        },
    };
    let Some(url) = located.url else {
        let facts = failure_facts(
            Some(String::from(
                "Failed to get upload URL from initial request",
            )),
            String::new(),
        );
        return Err(UploadError::Provider(handle_error(&facts, None)));
    };
    Ok(UploadTarget {
        url,
        data: located.data,
    })
}
