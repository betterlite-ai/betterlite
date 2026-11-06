pub mod api;
pub mod ids;
pub mod payload;
pub mod types;
pub mod utils;

mod test_support;
mod tests;

use std::error::Error as StdError;

use betterlite_config::loader::EnvironmentLookup;
use chrono::{DateTime, Utc};
use secrecy::SecretString;

use self::api::{Api, ApiError, HttpMethod};
use self::ids::UuidSource;
use self::payload::{CallbackKwargs, base_api_span, trace_api};
use self::types::{Environment, SpanTimes, TraceApi, TraceSpanApiStatus};
use self::utils::{EnvironmentError, to_zod_compatible_iso, validate_environment};
use crate::http_analytics_loggers::delivery::Delivery;

// The litellm.integrations.deepeval.DeepEvalLogger callback surface, without the response object it never reads.
#[derive(Clone, Copy, Debug)]
pub struct Call<'event> {
    pub kwargs: &'event CallbackKwargs,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

pub struct DeepEvalLogger<D, I> {
    api: Api<D>,
    environment: Environment,
    uuids: I,
}

#[derive(Debug, thiserror::Error)]
pub enum DeepEvalError<E>
where
    E: StdError + 'static,
{
    #[error("CONFIDENT_API_KEY is not set")]
    MissingApiKey,
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Api(#[from] ApiError<E>),
}

pub type DeepEvalResult<E> = Result<(), DeepEvalError<E>>;

impl<D: Delivery, I: UuidSource> DeepEvalLogger<D, I> {
    // DeepEvalLogger.__init__: LITELM_ENVIRONMENT is validated before CONFIDENT_API_KEY is required.
    pub fn from_environment<L>(
        lookup: &L,
        delivery: D,
        uuids: I,
    ) -> Result<Self, DeepEvalError<D::Error>>
    where
        L: EnvironmentLookup,
    {
        let name = lookup
            .lookup("LITELM_ENVIRONMENT")
            .unwrap_or_else(|| "development".to_owned());
        let environment = validate_environment(&name)?;
        let api_key = lookup
            .lookup("CONFIDENT_API_KEY")
            .filter(|key| !key.is_empty())
            .ok_or(DeepEvalError::MissingApiKey)?;
        Ok(Self {
            api: Api::new(&SecretString::from(api_key), delivery)?,
            environment,
            uuids,
        })
    }

    pub async fn log_success_event(&self, call: Call<'_>) -> DeepEvalResult<D::Error> {
        let trace = self.prepare_trace(call, TraceSpanApiStatus::Success);
        let response = self.api.send_request(HttpMethod::Post, &trace).await?;
        tracing::debug!(status = response.status, "DeepEvalLogger: sync sent");
        Ok(())
    }

    pub async fn log_failure_event(&self, call: Call<'_>) -> DeepEvalResult<D::Error> {
        let trace = self.prepare_trace(call, TraceSpanApiStatus::Errored);
        let response = self.api.send_request(HttpMethod::Post, &trace).await?;
        tracing::debug!(status = response.status, "DeepEvalLogger: sync failed");
        Ok(())
    }

    pub async fn async_log_success_event(&self, call: Call<'_>) -> DeepEvalResult<D::Error> {
        let trace = self.prepare_trace(call, TraceSpanApiStatus::Success);
        self.api.a_send_request(HttpMethod::Post, &trace).await?;
        tracing::debug!("DeepEvalLogger: async success sent");
        Ok(())
    }

    pub async fn async_log_failure_event(&self, call: Call<'_>) -> DeepEvalResult<D::Error> {
        let trace = self.prepare_trace(call, TraceSpanApiStatus::Errored);
        self.api.a_send_request(HttpMethod::Post, &trace).await?;
        tracing::debug!("DeepEvalLogger: async failure sent");
        Ok(())
    }

    fn prepare_trace(&self, call: Call<'_>, status: TraceSpanApiStatus) -> TraceApi {
        let start = to_zod_compatible_iso(&call.start_time);
        let end = to_zod_compatible_iso(&call.end_time);
        let times = SpanTimes {
            start: &start,
            end: &end,
        };
        let span = base_api_span(call.kwargs, times, status, &self.uuids);
        trace_api(span, times, self.environment)
    }
}
