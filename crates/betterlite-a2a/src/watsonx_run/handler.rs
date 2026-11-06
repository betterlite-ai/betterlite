use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::transform::ResponseMessage;
use crate::watsonx_credentials::bearer::{
    BearerTokens, CredentialsError, TokenClock, TokenTransport,
};
use crate::watsonx_credentials::params::{
    MissingParam, WxoLitellmParams, WxoRequestParams, extract_litellm_params,
};
use crate::watsonx_credentials::token_cache::TokenStore;
use crate::watsonx_transform::extract::{
    WxoSendParams, extract_text_from_a2a_params, extract_text_from_wxo_result,
};
use crate::watsonx_transform::fake_streaming::{WxoStreamStep, fake_streaming_from_text};
use crate::watsonx_transform::response::build_a2a_message_response;

use super::accumulator::{LineAccumulator, SseError, accumulate_text};
use super::event::RunEvent;
use super::poll::{PollError, poll_run};
use super::run_api::{ApiError, Method, RunBody, RunContent, RunMessage, call_run};
use super::transport::{RunLimits, RunRequest, RunTransport, StreamBody};

mod tests;

const JSON_ACCEPT: &str = "application/json";
const STREAM_ACCEPT: &str = "text/event-stream, application/json";

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
    runs_url: String,
    stream_url: String,
    headers: BTreeMap<String, String>,
    body: RunBody,
}

pub struct RunHandler<T, K, C, S, U> {
    pub transport: T,
    pub credentials: BearerTokens<K, C, S>,
    pub source: U,
    pub max_response_bytes: usize,
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
        let prepared = self.prepare(params, raw, JSON_ACCEPT).await?;
        let limits = RunLimits {
            timeout: NON_STREAMING_TIMEOUT,
            max_response_bytes: self.max_response_bytes,
        };
        let request = RunRequest {
            url: &prepared.runs_url,
            headers: &prepared.headers,
            body: Some(&prepared.body),
            limits,
        };
        let started = call_run(&self.transport, &request, Method::Post).await?;
        let run = poll_run(
            &self.transport,
            &prepared.runs_url,
            started,
            &prepared.headers,
            limits,
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
        let prepared = self.prepare(params, raw, STREAM_ACCEPT).await?;
        let text = match self.open_stream(&prepared).await {
            Ok(text) => text,
            Err(RunError::Api(ApiError::Transport(_))) => {
                let response = self.handle_non_streaming(request_id, params, raw).await?;
                response_text(&response)
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
        Ok(planned(&wxo, params, &token, accept))
    }

    async fn open_stream(
        &self,
        prepared: &Prepared,
    ) -> Result<String, RunError<T::Error, K::Error>> {
        let limits = RunLimits {
            timeout: STREAMING_TIMEOUT,
            max_response_bytes: self.max_response_bytes,
        };
        let (content_type, chunks) = self.read_stream(prepared, limits).await?;
        if content_type.to_lowercase().contains("text/event-stream") {
            return Ok(sse_text(&chunks)?);
        }
        let event =
            serde_json::from_slice::<RunEvent>(&chunks.concat()).map_err(ApiError::Decode)?;
        let run = poll_run(
            &self.transport,
            &prepared.runs_url,
            event,
            &prepared.headers,
            limits,
        )
        .await?;
        Ok(extract_text_from_wxo_result(&run.run))
    }

    async fn read_stream(
        &self,
        prepared: &Prepared,
        limits: RunLimits,
    ) -> Result<(String, Vec<Vec<u8>>), RunError<T::Error, K::Error>> {
        let request = RunRequest {
            url: &prepared.stream_url,
            headers: &prepared.headers,
            body: Some(&prepared.body),
            limits,
        };
        let response = self
            .transport
            .post_stream(&request)
            .await
            .map_err(ApiError::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(RunError::Api(ApiError::Status {
                status: response.status,
            }));
        }
        let content_type = response.content_type;
        let chunks = match response.body {
            StreamBody::Chunks(chunks) => chunks,
            StreamBody::ReadFailed(error) => return Err(RunError::Body(error)),
            StreamBody::TooLarge { max } => return Err(RunError::Api(ApiError::TooLarge { max })),
        };
        let mut total = 0_usize;
        for chunk in &chunks {
            total = total.saturating_add(chunk.len());
        }
        if total > limits.max_response_bytes {
            return Err(RunError::Api(ApiError::TooLarge {
                max: limits.max_response_bytes,
            }));
        }
        Ok((content_type, chunks))
    }
}

fn planned(
    wxo: &WxoRequestParams,
    params: &WxoSendParams,
    token: &SecretString,
    accept: &str,
) -> Prepared {
    let base = format!(
        "{}/orchestrate/cpd/instances/{}",
        wxo.cp4d_host.trim_end_matches('/'),
        wxo.instance_id
    );
    let body = RunBody {
        agent_id: wxo.wxo_agent_id.clone(),
        message: RunMessage {
            role: "user",
            content: vec![RunContent {
                response_type: "text",
                text: extract_text_from_a2a_params(params),
            }],
        },
        thread_id: wxo.thread_id.clone(),
    };
    Prepared {
        runs_url: format!("{base}/v1/orchestrate/runs"),
        stream_url: format!("{base}/v1/orchestrate/runs/stream"),
        headers: BTreeMap::from([
            (
                "Authorization".to_owned(),
                format!("Bearer {}", token.expose_secret()),
            ),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Accept".to_owned(), accept.to_owned()),
        ]),
        body,
    }
}

fn response_text(response: &Event<ResponseMessage>) -> String {
    for part in &response.result.parts {
        if part.kind.as_deref() != Some("text") {
            continue;
        }
        let Some(text) = part.text.as_deref().filter(|text| !text.is_empty()) else {
            continue;
        };
        return text.to_owned();
    }
    String::new()
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
