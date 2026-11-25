use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::responses_create::bridge::Created;
use betterlite_core::api::responses_stream_driver::source::{PayloadSource, SourceFailure};
use betterlite_core::api::responses_stream_events::ProjectQuotaCallback;
use betterlite_core::api::responses_ws_history::provider::{
    ProviderResolution, ProviderResolutionFailure,
};
use betterlite_core::api::responses_ws_managed::provider::{
    ProviderCallError, ProviderChunk, ResponsesStream, ResponsesStreaming,
};
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::fixtures::{AdapterError, Outbound, dispatch, provider_chunk};
use super::input::{QuotaRow, rows::QuotaKind};
use super::support::{RecordedCall, RecordedQuota, Shared, lock};

pub(super) struct HttpStream {
    shared: Shared,
    source: Box<dyn PayloadSource>,
}

#[async_trait]
impl ResponsesStream for HttpStream {
    type Error = AdapterError;

    async fn next_chunk(
        &mut self,
    ) -> Result<Option<ProviderChunk>, ProviderCallError<AdapterError>> {
        match self.source.next_payload().await {
            Some(Ok(bytes)) => {
                let chunk = provider_chunk(&bytes).map_err(ProviderCallError::Stream)?;
                if chunk.is_none() {
                    lock(&self.shared).trace.push("provider-end".to_owned());
                }
                Ok(chunk)
            }
            Some(Err(SourceFailure::Transport(source))) => {
                Err(ProviderCallError::Stream(AdapterError::Transport(source)))
            }
            Some(Err(SourceFailure::Decode(message))) => {
                Err(ProviderCallError::Stream(AdapterError::Decode { message }))
            }
            None => {
                lock(&self.shared).trace.push("provider-end".to_owned());
                Ok(None)
            }
        }
    }
}

pub(super) struct ScriptedProvider {
    pub(super) shared: Shared,
    pub(super) outbound: Outbound,
    pub(super) base_url: String,
}

#[async_trait]
impl ResponsesStreaming for ScriptedProvider {
    type Error = AdapterError;
    type Stream = HttpStream;

    async fn stream(
        &self,
        model: &str,
        call_kwargs: &IndexMap<String, Json>,
    ) -> Result<HttpStream, ProviderCallError<AdapterError>> {
        {
            let mut recorder = lock(&self.shared);
            recorder.trace.push("provider".to_owned());
            recorder.calls.push(RecordedCall {
                model: model.to_owned(),
                kwargs: call_kwargs.clone(),
            });
        };
        let created = dispatch(&self.outbound, &self.base_url, model, call_kwargs)
            .await
            .map_err(|source| ProviderCallError::Call(AdapterError::Dispatch(Box::new(source))))?;
        match created {
            Created::Live(live) => Ok(HttpStream {
                shared: Shared::clone(&self.shared),
                source: live.source,
            }),
            Created::Responses(_) | Created::Raw(_) | Created::Synthetic(_) | Created::Chat(_) => {
                Err(ProviderCallError::Stream(AdapterError::NotStreaming))
            }
        }
    }
}

pub(super) struct ScriptedQuota {
    pub(super) shared: Shared,
    pub(super) row: QuotaRow,
}

#[async_trait]
impl ProjectQuotaCallback for ScriptedQuota {
    async fn enforce_frame_quota(
        &self,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) -> Result<(), ErrorClass> {
        let mut recorder = lock(&self.shared);
        recorder.trace.push("quota".to_owned());
        recorder.quotas.push(RecordedQuota {
            model: requested_model.map(str::to_owned),
            input_tokens: estimated_input_tokens,
            output_tokens: estimated_output_tokens,
        });
        drop(recorder);
        match self.row.kind {
            QuotaKind::RateLimit => {
                let message = format!("litellm.RateLimitError: {}", self.row.message);
                Err(rate_limited(&message))
            }
            QuotaKind::Internal => Err(ErrorClass::InternalServer(Box::new(context(
                &self.row.message,
            )))),
        }
    }
}

fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn rate_limited(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(message),
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

pub(super) struct ScriptedResolver;

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, model: &str) -> Result<String, ProviderResolutionFailure> {
        match model.split_once('/') {
            Some((provider @ ("openai" | "anthropic" | "vertex_ai"), _rest)) => {
                Ok(provider.to_owned())
            }
            _ => Err(ProviderResolutionFailure {
                message: format!("no provider answers {model}"),
            }),
        }
    }
}
