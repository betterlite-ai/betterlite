use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;
use chrono::{DateTime, FixedOffset};
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::response::Logging;

use super::CUSTOM_LLM_PROVIDER;
use super::config::{AnthropicBatchesConfig, BatchesConfig, Environment};
use super::decode::decoded_text;
use super::error::BatchesError;
use super::timestamps::Clock;

// B173's pooled client in production, a recorded reply in tests and in the oracle helper
pub trait BatchTransport: Sync {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl BatchTransport for ProxiedClient {
    async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        Self::send_buffered(self, request).await
    }
}

pub struct RetrieveRequest<'call> {
    pub batch_id: &'call str,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    // the source accepts the call timeout and retry count but never forwards them to the client
    pub timeout: Option<Duration>,
    pub max_retries: Option<i64>,
}

#[derive(Debug)]
pub struct AnthropicBatchesHandler<T> {
    pub transport: T,
    pub config: AnthropicBatchesConfig,
    pub clock: Clock,
    pub request_timeout: Duration,
}

impl<T: BatchTransport> AnthropicBatchesHandler<T> {
    pub async fn retrieve_batch(
        &self,
        request: &RetrieveRequest<'_>,
        logging: Logging<'_>,
    ) -> Result<BatchObject, BatchesError> {
        let api_base = request
            .api_base
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.config.settings.api_base);
        // only the API key counts here, so an auth token alone reaches the validator and never this check
        let api_key = request
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.config.settings.api_key.as_ref())
            .ok_or(BatchesError::MissingApiKey)?;
        let url = self
            .config
            .get_retrieve_batch_url(Some(api_base), request.batch_id)?;
        let mut headers = HeaderMap::new();
        self.config.validate_environment(
            &mut headers,
            Environment {
                api_key: Some(api_key),
                api_base: Some(api_base),
                params_api_base: None,
            },
        )?;
        let Logging { state, env } = logging;
        pre_call(
            state,
            &PreCall {
                input: Json::Text(request.batch_id.to_owned()),
                model: None,
                additional_args: additional_args(&url, &headers),
            },
            env,
        )
        .map_err(|source| BatchesError::Logging { source })?;
        let outbound = OutboundRequest {
            method: Method::GET,
            url: Url::parse(&url).map_err(|source| BatchesError::InvalidBase { source })?,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.request_timeout,
        };
        let reply = self
            .transport
            .send_buffered(outbound)
            .await
            .map_err(|source| BatchesError::Transport { source })?;
        if reply.status >= 400 {
            // raise_for_status reads the body as text, so an invalid byte shows as U+FFFD
            let message = decoded_text(&reply.body);
            return Err(BatchesError::provider(
                &message,
                reply.status,
                Some(&reply.headers),
            ));
        }
        self.config
            .transform_retrieve_batch_response(&reply.body, &self.clock)
            .map_err(BatchesError::Decode)
    }
}

// httpx decodes header bytes as latin-1, so a byte that is not ASCII maps to the char of the same value
fn header_text(value: &HeaderValue) -> String {
    value.to_str().map_or_else(
        |_invalid| {
            value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect()
        },
        str::to_owned,
    )
}

fn additional_args(url: &str, headers: &HeaderMap) -> AdditionalArgs {
    let fields: IndexMap<String, Json> = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), Json::Text(header_text(value))))
        .collect();
    AdditionalArgs {
        api_base: Some(url.to_owned()),
        headers: Some(fields),
        complete_input_dict: Some(RawBody::Object(IndexMap::new())),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}

// the logging object LiteLLM builds when the call passes none, with its start_time left to the caller's clock
#[derive(Debug)]
pub struct BatchLogging {
    pub model: String,
    pub messages: Vec<Json>,
    pub stream: bool,
    pub call_type: String,
    pub start_time: Option<DateTime<FixedOffset>>,
    pub litellm_call_id: String,
    pub function_id: String,
}

#[must_use]
pub fn default_batch_logging(batch_id: &str) -> BatchLogging {
    BatchLogging {
        // the config's custom_llm_provider, with no model the batch flow knows
        model: format!("{CUSTOM_LLM_PROVIDER}/unknown"),
        messages: Vec::new(),
        stream: false,
        call_type: "batch_retrieve".to_owned(),
        start_time: None,
        litellm_call_id: format!("batch_retrieve_{batch_id}"),
        function_id: "batch_retrieve".to_owned(),
    }
}

impl BatchLogging {
    #[must_use]
    pub fn into_init(self, now: DateTime<FixedOffset>) -> LoggingInit {
        LoggingInit {
            model: self.model,
            call_type: self.call_type,
            start_time: self.start_time.unwrap_or(now),
            litellm_call_id: self.litellm_call_id,
            function_id: self.function_id,
            messages: Some(self.messages),
            stream: Some(self.stream),
            litellm_trace_id: None,
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        }
    }
}
