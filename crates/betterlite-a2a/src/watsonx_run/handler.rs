use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use secrecy::ExposeSecret as _;
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::transform::ResponseMessage;
use crate::watsonx_credentials::bearer::{
    BearerTokens, CredentialsError, TokenClock, TokenTransport,
};
use crate::watsonx_credentials::params::{MissingParam, WxoLitellmParams, extract_litellm_params};
use crate::watsonx_credentials::token_cache::TokenStore;
use crate::watsonx_transform::extract::{
    WxoSendParams, extract_text_from_a2a_params, extract_text_from_wxo_result,
};
use crate::watsonx_transform::fake_streaming::{WxoStreamStep, fake_streaming_from_text};
use crate::watsonx_transform::response::build_a2a_message_response;

use super::accumulator::{LineAccumulator, SseError, accumulate_text};
use super::event::RunEvent;
use super::poll::{PollError, successful_run};
use super::run_api::{ApiError, Method, RunBody, RunContent, RunMessage, call_run};
use super::transport::{RunRequest, RunTransport, StreamBody};

mod tests;

pub const NON_STREAMING_TIMEOUT: Duration = Duration::from_secs(90);
pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum RunError<E, F>
where
    E: Error + 'static,
    F: Error + 'static,
{
    #[error(transparent)]
    Params(#[from] MissingParam),
    #[error("the watsonx Orchestrate bearer token is unavailable")]
    Credentials(#[source] CredentialsError<F>),
    #[error(transparent)]
    Api(#[from] ApiError<E>),
    #[error(transparent)]
    Poll(#[from] PollError<E>),
    #[error(transparent)]
    Sse(#[from] SseError),
    #[error("the watsonx Orchestrate stream body could not be read")]
    Body(#[source] E),
}

struct Prepared {
    base_url: String,
    headers: BTreeMap<String, String>,
    body: RunBody,
}

pub struct RunHandler<T, K, C, S, U> {
    pub transport: T,
    pub credentials: BearerTokens<K, C, S>,
    pub source: U,
}

impl<T, K, C, S, U> RunHandler<T, K, C, S, U>
where
    T: RunTransport,
    K: TokenTransport,
    C: TokenClock,
    S: TokenStore,
    U: EventSource + Send + Sync,
{
    pub async fn handle_non_streaming(
        &mut self,
        request_id: &str,
        params: &WxoSendParams,
        raw: &WxoLitellmParams,
    ) -> Result<Event<ResponseMessage>, RunError<T::Error, K::Error>> {
        let prepared = self.prepare(params, raw, "application/json").await?;
        let url = format!("{}/v1/orchestrate/runs", prepared.base_url);
        let request = RunRequest {
            url: &url,
            headers: &prepared.headers,
            body: Some(&prepared.body),
            timeout: NON_STREAMING_TIMEOUT,
        };
        let started = call_run(&self.transport, &request, Method::Post).await?;
        let run = successful_run(
            &self.transport,
            started,
            &prepared.base_url,
            &prepared.headers,
            NON_STREAMING_TIMEOUT,
        )
        .await?;
        let text = extract_text_from_wxo_result(&run.run);
        Ok(build_a2a_message_response(
            request_id,
            &text,
            &mut self.source,
        ))
    }

    pub async fn handle_streaming(
        &mut self,
        request_id: &str,
        params: &WxoSendParams,
        raw: &WxoLitellmParams,
        pacing: StreamPacing,
    ) -> Result<Vec<WxoStreamStep>, RunError<T::Error, K::Error>> {
        let prepared = self
            .prepare(params, raw, "text/event-stream, application/json")
            .await?;
        let url = format!("{}/v1/orchestrate/runs/stream", prepared.base_url);
        let request = RunRequest {
            url: &url,
            headers: &prepared.headers,
            body: Some(&prepared.body),
            timeout: STREAMING_TIMEOUT,
        };
        let text = match self.open_stream(&request, &prepared).await {
            Ok(text) => text,
            Err(RunError::Api(ApiError::Transport(_))) => {
                self.fallback_text(request_id, params, raw).await?
            }
            Err(error) => return Err(error),
        };
        Ok(fake_streaming_from_text(
            &text,
            request_id,
            pacing,
            &mut self.source,
        ))
    }

    async fn prepare(
        &self,
        params: &WxoSendParams,
        raw: &WxoLitellmParams,
        accept: &str,
    ) -> Result<Prepared, RunError<T::Error, K::Error>> {
        let wxo = extract_litellm_params(raw.clone())?;
        let token = self
            .credentials
            .bearer_token(&wxo)
            .await
            .map_err(RunError::Credentials)?;
        Ok(Prepared {
            base_url: format!(
                "{}/orchestrate/cpd/instances/{}",
                wxo.cp4d_host.trim_end_matches('/'),
                wxo.instance_id
            ),
            headers: BTreeMap::from([
                (
                    "Authorization".to_owned(),
                    format!("Bearer {}", token.expose_secret()),
                ),
                ("Content-Type".to_owned(), "application/json".to_owned()),
                ("Accept".to_owned(), accept.to_owned()),
            ]),
            body: RunBody {
                agent_id: wxo.wxo_agent_id,
                message: RunMessage {
                    role: "user",
                    content: vec![RunContent {
                        response_type: "text",
                        text: extract_text_from_a2a_params(params),
                    }],
                },
                thread_id: wxo.thread_id,
            },
        })
    }

    async fn open_stream(
        &self,
        request: &RunRequest<'_>,
        prepared: &Prepared,
    ) -> Result<String, RunError<T::Error, K::Error>> {
        let response = self
            .transport
            .post_stream(request)
            .await
            .map_err(ApiError::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(RunError::Api(ApiError::Status {
                status: response.status,
            }));
        }
        let chunks = match response.body {
            StreamBody::Chunks(chunks) => chunks,
            StreamBody::ReadFailed(error) => return Err(RunError::Body(error)),
        };
        if response
            .content_type
            .to_lowercase()
            .contains("text/event-stream")
        {
            return Ok(sse_text(&chunks)?);
        }
        let event =
            serde_json::from_slice::<RunEvent>(&chunks.concat()).map_err(ApiError::Decode)?;
        let run = successful_run(
            &self.transport,
            event,
            &prepared.base_url,
            &prepared.headers,
            STREAMING_TIMEOUT,
        )
        .await?;
        Ok(extract_text_from_wxo_result(&run.run))
    }

    async fn fallback_text(
        &mut self,
        request_id: &str,
        params: &WxoSendParams,
        raw: &WxoLitellmParams,
    ) -> Result<String, RunError<T::Error, K::Error>> {
        let response = self.handle_non_streaming(request_id, params, raw).await?;
        let text = response
            .result
            .parts
            .iter()
            .find(|part| {
                part.kind.as_deref() == Some("text")
                    && part.text.as_deref().is_some_and(|text| !text.is_empty())
            })
            .and_then(|part| part.text.clone());
        Ok(text.unwrap_or_default())
    }
}

fn sse_text(chunks: &[Vec<u8>]) -> Result<String, SseError> {
    let mut accumulator = LineAccumulator::default();
    let mut lines = Vec::new();
    for chunk in chunks {
        lines.extend(accumulator.push(chunk)?);
    }
    if let Some(line) = accumulator.finish() {
        lines.push(line);
    }
    accumulate_text(&lines)
}
