use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use serde_with::{DurationSecondsWithFrac, serde_as};
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_handler::{HandlerError, HandlerRequest, PydanticAIHandler};
use crate::pydantic_ai_submit::{AgentTransport, DEFAULT_TIMEOUT};
use crate::pydantic_ai_transformation::fake_streaming::{SimulatedEvent, StreamPacing};
use crate::pydantic_ai_transformation::transform::ResponseMessage;

mod tests;

#[derive(Debug, Error)]
pub enum ProviderError<E>
where
    E: StdError + 'static,
{
    #[error("the Pydantic AI provider arguments are invalid")]
    Kwargs(#[source] serde_json::Error),
    #[error(transparent)]
    Handler(#[from] HandlerError<E>),
}

pub struct ProviderCall<'call, P> {
    pub request_id: &'call str,
    pub params: &'call P,
    pub api_base: Option<&'call str>,
    pub kwargs: &'call BTreeMap<String, AgentJson>,
}

#[serde_as]
#[derive(Deserialize)]
struct ProviderKwargs {
    #[serde(default)]
    #[serde_as(as = "Option<DurationSecondsWithFrac<f64>>")]
    timeout: Option<Duration>,
    #[serde(default)]
    agent_extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    chunk_size: Option<NonZeroUsize>,
    #[serde(default)]
    delay_ms: Option<u64>,
}

pub struct PydanticAIProviderConfig<T> {
    pub handler: PydanticAIHandler<T>,
}

struct ResolvedKwargs {
    timeout: Duration,
    headers: BTreeMap<String, String>,
    pacing: StreamPacing,
}

impl<T> PydanticAIProviderConfig<T>
where
    T: AgentTransport,
{
    pub async fn handle_non_streaming<P, S>(
        &self,
        call: &ProviderCall<'_, P>,
        source: &mut S,
    ) -> Result<Event<ResponseMessage>, ProviderError<T::Error>>
    where
        P: Serialize + Sync,
        S: EventSource,
    {
        let kwargs = read_kwargs(call.kwargs)?;
        let request = HandlerRequest {
            request_id: call.request_id,
            params: call.params,
            api_base: call.api_base,
            timeout: kwargs.timeout,
            headers: &kwargs.headers,
        };
        Ok(self.handler.handle_non_streaming(&request, source).await?)
    }

    // An empty api_base is refused here and not in the non-streaming path, as LiteLLM does
    pub async fn handle_streaming<P, S>(
        &self,
        call: &ProviderCall<'_, P>,
        source: &mut S,
    ) -> Result<Vec<SimulatedEvent>, ProviderError<T::Error>>
    where
        P: Serialize + Sync,
        S: EventSource,
    {
        if call.api_base.is_none_or(str::is_empty) {
            return Err(ProviderError::Handler(HandlerError::ApiBaseRequired));
        }
        let kwargs = read_kwargs(call.kwargs)?;
        let request = HandlerRequest {
            request_id: call.request_id,
            params: call.params,
            api_base: call.api_base,
            timeout: kwargs.timeout,
            headers: &kwargs.headers,
        };
        Ok(self
            .handler
            .handle_streaming(&request, kwargs.pacing, source)
            .await?)
    }
}

fn read_kwargs<E>(kwargs: &BTreeMap<String, AgentJson>) -> Result<ResolvedKwargs, ProviderError<E>>
where
    E: StdError + 'static,
{
    let fields = AgentJson::Object(kwargs.clone().into_iter().collect());
    let parsed = ProviderKwargs::deserialize(fields).map_err(ProviderError::Kwargs)?;
    let defaults = StreamPacing::default();
    Ok(ResolvedKwargs {
        timeout: parsed.timeout.unwrap_or(DEFAULT_TIMEOUT),
        headers: parsed.agent_extra_headers.unwrap_or_default(),
        pacing: StreamPacing {
            chunk_size: parsed.chunk_size.unwrap_or(defaults.chunk_size),
            delay: parsed
                .delay_ms
                .map_or(defaults.delay, Duration::from_millis),
        },
    })
}
