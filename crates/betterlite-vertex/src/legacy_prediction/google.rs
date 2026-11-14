use std::collections::VecDeque;
use std::pin::Pin;
use std::str;
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt as _};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::model_urls::base_url::get_vertex_base_url;

use super::client::{
    EndpointPredictRequest, PredictionClient, PredictionStream, PublisherPredictRequest,
    chat_predict_body, chat_streaming_body, endpoint_predict_body, text_predict_body,
    text_streaming_body,
};
use super::error::PredictionError;
use super::frames::PredictionFrames;
use super::records::{
    ChunkKind, LegacyChunk, PredictionsReply, TextReply, chat_reply, decode_legacy_prediction,
    text_reply,
};
use super::tensor::chunk_of;

pub const API_VERSION: &str = "v1";

// The PredictionService RPCs the legacy SDK objects call, over B173's pooled transport
pub struct GooglePredictionClient {
    pub transport: ProxiedClient,
    pub deadline: Duration,
}

impl GooglePredictionClient {
    async fn publisher_reply(
        &self,
        request: &PublisherPredictRequest,
        body: Vec<u8>,
    ) -> Result<PredictionsReply, PredictionError> {
        let resource = format!(
            "projects/{}/locations/{}/publishers/google/models/{}",
            request.project, request.location, request.model
        );
        let url = rpc_url(&request.location, &resource, "predict")?;
        let reply = self.buffered(&request.token, url, body).await?;
        decode_legacy_prediction(&reply)
    }

    async fn publisher_stream(
        &self,
        request: &PublisherPredictRequest,
        body: Vec<u8>,
        kind: ChunkKind,
    ) -> Result<PredictionStream, PredictionError> {
        let resource = format!(
            "projects/{}/locations/{}/publishers/google/models/{}",
            request.project, request.location, request.model
        );
        let url = rpc_url(&request.location, &resource, "serverStreamingPredict")?;
        self.streaming(&request.token, url, body, kind).await
    }

    async fn buffered(
        &self,
        token: &SecretString,
        url: Url,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, PredictionError> {
        let request = rpc_request(token, url, body, self.deadline)?;
        let response = self
            .transport
            .send_following(request)
            .await
            .map_err(|source| PredictionError::Transport { source })?;
        let buffered = response
            .buffered(self.transport.limits.body_bytes)
            .await
            .map_err(|source| PredictionError::Transport { source })?;
        success_body(buffered)
    }

    async fn streaming(
        &self,
        token: &SecretString,
        url: Url,
        body: Vec<u8>,
        kind: ChunkKind,
    ) -> Result<PredictionStream, PredictionError> {
        let request = rpc_request(token, url, body, self.deadline)?;
        let response = self
            .transport
            .send_following(request)
            .await
            .map_err(|source| PredictionError::Transport { source })?;
        let status = response.response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(PredictionChunks {
                body: response.body_stream(),
                frames: PredictionFrames::default(),
                kind,
                pending: VecDeque::new(),
                finished: false,
            }
            .boxed());
        }
        let buffered = response
            .buffered(self.transport.limits.body_bytes)
            .await
            .map_err(|source| PredictionError::Transport { source })?;
        Err(PredictionError::Http {
            status,
            message: message_of(&buffered.body),
        })
    }
}

impl PredictionClient for GooglePredictionClient {
    async fn send_message_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> Result<TextReply, PredictionError> {
        let body = chat_predict_body(&request.prompt, &request.options)?;
        let reply = self.publisher_reply(request, body).await?;
        chat_reply(&reply)
    }

    async fn predict_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> Result<TextReply, PredictionError> {
        let body = text_predict_body(&request.prompt, &request.options)?;
        let reply = self.publisher_reply(request, body).await?;
        text_reply(&reply)
    }

    async fn predict_endpoint(
        &self,
        request: &EndpointPredictRequest,
    ) -> Result<PredictionsReply, PredictionError> {
        let body = endpoint_predict_body(&request.instances)?;
        let url = rpc_url(&request.location, &request.endpoint, "predict")?;
        let reply = self.buffered(&request.token, url, body).await?;
        decode_legacy_prediction(&reply)
    }

    async fn send_message_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> Result<PredictionStream, PredictionError> {
        let body = chat_streaming_body(&request.prompt, &request.options)?;
        self.publisher_stream(request, body, ChunkKind::Chat).await
    }

    async fn predict_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> Result<PredictionStream, PredictionError> {
        let body = text_streaming_body(&request.prompt, &request.options)?;
        self.publisher_stream(request, body, ChunkKind::Text).await
    }
}

fn rpc_request(
    token: &SecretString,
    url: Url,
    body: Vec<u8>,
    deadline: Duration,
) -> Result<OutboundRequest, PredictionError> {
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: auth_headers(token)?,
        body: RequestBody::Json(body),
        deadline,
    })
}

fn auth_headers(token: &SecretString) -> Result<HeaderMap, PredictionError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut value =
        HeaderValue::from_str(&format!("Bearer {}", token.expose_secret())).map_err(|source| {
            PredictionError::Header {
                source: http::Error::from(source),
            }
        })?;
    value.set_sensitive(true);
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

fn rpc_url(location: &str, resource: &str, method: &str) -> Result<Url, PredictionError> {
    let base = get_vertex_base_url(Some(location))
        .map_err(|source| PredictionError::VertexUrl { source })?;
    Url::parse(&format!("{base}/{API_VERSION}/{resource}:{method}"))
        .map_err(|source| PredictionError::Url { source })
}

fn success_body(response: UpstreamResponse) -> Result<Vec<u8>, PredictionError> {
    if (200..300).contains(&response.status) {
        return Ok(response.body);
    }
    Err(PredictionError::Http {
        status: response.status,
        message: message_of(&response.body),
    })
}

fn message_of(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

// Owns the leased body stream, so dropping the prediction stream closes its transport
struct PredictionChunks {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    frames: PredictionFrames,
    kind: ChunkKind,
    pending: VecDeque<Result<LegacyChunk, PredictionError>>,
    finished: bool,
}

impl Stream for PredictionChunks {
    type Item = Result<LegacyChunk, PredictionError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = this.pending.pop_front() {
                return Poll::Ready(Some(item));
            }
            if this.finished {
                return Poll::Ready(None);
            }
            match this.body.as_mut().poll_next(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    this.finished = true;
                    if let Err(error) = this.frames.finish() {
                        return Poll::Ready(Some(Err(error)));
                    }
                }
                Poll::Ready(Some(Err(source))) => {
                    this.finished = true;
                    return Poll::Ready(Some(Err(PredictionError::Read { source })));
                }
                Poll::Ready(Some(Ok(bytes))) => match this.frames.push(&bytes) {
                    Ok(objects) => {
                        for object in objects {
                            this.pending.push_back(chunk_of(this.kind, &object));
                        }
                    }
                    Err(error) => {
                        this.finished = true;
                        return Poll::Ready(Some(Err(error)));
                    }
                },
            }
        }
    }
}

mod tests;
