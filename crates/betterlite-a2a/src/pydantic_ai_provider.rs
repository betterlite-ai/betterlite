use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;
use serde_with::{DurationSecondsWithFrac, serde_as};
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::pydantic_ai_handler::{HandlerError, HandlerRequest, PydanticAIHandler};
use crate::pydantic_ai_submit::{AgentTransport, DEFAULT_TIMEOUT};
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::send::Delivery;

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

pub struct ProviderCall<'call> {
    pub request_id: &'call str,
    pub params: &'call AgentJson,
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

pub struct PydanticAIProviderConfig<T, S> {
    pub handler: PydanticAIHandler<T, S>,
}

struct ResolvedKwargs {
    timeout: Duration,
    headers: BTreeMap<String, String>,
    pacing: StreamPacing,
}

impl ResolvedKwargs {
    const fn request<'call>(&'call self, call: &ProviderCall<'call>) -> HandlerRequest<'call> {
        HandlerRequest {
            request_id: call.request_id,
            params: call.params,
            api_base: call.api_base,
            timeout: self.timeout,
            headers: &self.headers,
        }
    }
}

impl<T, S> PydanticAIProviderConfig<T, S>
where
    T: AgentTransport,
    S: EventSource,
{
    pub async fn handle<D>(
        &mut self,
        call: &ProviderCall<'_>,
    ) -> Result<D::Output, ProviderError<T::Error>>
    where
        D: Delivery,
    {
        if D::REFUSES_EMPTY_API_BASE && call.api_base.is_none_or(str::is_empty) {
            return Err(ProviderError::Handler(HandlerError::ApiBaseRequired));
        }
        let kwargs = read_kwargs(call.kwargs)?;
        let request = kwargs.request(call);
        let delivery = D::from(kwargs.pacing);
        Ok(self.handler.handle(&request, delivery).await?)
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
