pub mod callbacks;
pub mod correlation;
pub mod events;
pub mod pricing;
pub mod types;
pub mod updates;

use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use std::fmt;
use thiserror::Error;

use self::correlation::{CorrelationOwnership, CorrelationSetup, apply_correlation};
use self::types::{
    CallbackPolicy, CostState, DeferredLogging, DynamicCallbacks, Messages, RequestLoggingFlags,
};

mod tests;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingInit {
    pub model: String,
    pub call_type: String,
    pub start_time: DateTime<FixedOffset>,
    pub litellm_call_id: String,
    pub function_id: String,
    #[serde(default)]
    pub messages: Option<Messages>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub litellm_trace_id: Option<String>,
    #[serde(default)]
    pub kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub litellm_params: IndexMap<String, Json>,
    #[serde(default, rename = "standard_callback_dynamic_params")]
    pub callback_dynamic_params: IndexMap<String, Json>,
    #[serde(default)]
    pub trusted_callback_vars: Vec<(String, SecretString)>,
    #[serde(default)]
    pub standard_built_in_tools_params: IndexMap<String, Json>,
    #[serde(default)]
    pub dynamic_callbacks: DynamicCallbacks,
    #[serde(default)]
    pub applied_guardrails: Option<Vec<String>>,
    #[serde(default)]
    pub request_logging: RequestLoggingFlags,
    #[serde(default)]
    pub callback_policy: CallbackPolicy,
}

pub struct LoggingState {
    pub model: String,
    pub messages: Option<Vec<Json>>,
    pub stream: Option<bool>,
    pub start_time: DateTime<FixedOffset>,
    pub call_type: String,
    pub litellm_call_id: String,
    pub litellm_trace_id: String,
    pub litellm_session_id: String,
    pub function_id: String,
    pub litellm_params: IndexMap<String, Json>,
    pub optional_params: IndexMap<String, Json>,
    pub user: Option<String>,
    pub litellm_request_debug: bool,
    pub stream_options: Option<Json>,
    pub cost: CostState,
    pub custom_llm_provider: Option<String>,
    pub model_call_details: IndexMap<String, Json>,
    pub response_timing_metrics: IndexMap<String, f64>,
    pub completion_start_time: Option<DateTime<FixedOffset>>,
    pub dynamic_callbacks: DynamicCallbacks,
    pub callback_dynamic_params: IndexMap<String, Json>,
    pub trusted_callback_vars: Vec<(String, SecretString)>,
    pub standard_built_in_tools_params: IndexMap<String, Json>,
    pub applied_guardrails: Option<Vec<String>>,
    pub request_logging: RequestLoggingFlags,
    pub truncated_messages_for_logging: Option<Json>,
    pub cost_breakdown: Option<CostBreakdown>,
    pub billed_token_rates: Option<BilledTokenRates>,
    pub correlation: CorrelationOwnership,
    pub deferred: DeferredLogging,
    pub callback_policy: CallbackPolicy,
}

// Params, litellm_params and trusted variables carry credentials, so only identifiers are printed
impl fmt::Debug for LoggingInit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoggingInit")
            .field("model", &self.model)
            .field("call_type", &self.call_type)
            .field("litellm_call_id", &self.litellm_call_id)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for LoggingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoggingState")
            .field("model", &self.model)
            .field("call_type", &self.call_type)
            .field("litellm_call_id", &self.litellm_call_id)
            .field("litellm_trace_id", &self.litellm_trace_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoggingStateError {
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
}

impl LoggingState {
    pub fn new<Id>(
        init: LoggingInit,
        mut correlation: CorrelationSetup<'_>,
        redactor: &Redactor,
        next_trace_id: Id,
    ) -> Result<Self, LoggingStateError>
    where
        Id: FnOnce() -> String,
    {
        let session_id = session_id_from_kwargs(&init.kwargs)?;
        let trace_id = init.litellm_trace_id.clone().unwrap_or_else(next_trace_id);
        let messages = init.messages.as_ref().map(Messages::normalized);
        let ownership = apply_correlation(&mut correlation, &trace_id, &session_id, redactor);
        let model_call_details = initial_model_call_details(&init, &trace_id);
        Ok(Self {
            model: init.model,
            messages,
            stream: init.stream,
            start_time: init.start_time,
            call_type: init.call_type,
            litellm_call_id: init.litellm_call_id,
            litellm_trace_id: trace_id,
            litellm_session_id: session_id,
            function_id: init.function_id,
            litellm_params: init.litellm_params,
            optional_params: IndexMap::new(),
            user: None,
            litellm_request_debug: false,
            stream_options: None,
            cost: CostState::default(),
            custom_llm_provider: None,
            model_call_details,
            response_timing_metrics: IndexMap::new(),
            completion_start_time: None,
            dynamic_callbacks: init.dynamic_callbacks,
            callback_dynamic_params: init.callback_dynamic_params,
            trusted_callback_vars: init.trusted_callback_vars,
            standard_built_in_tools_params: init.standard_built_in_tools_params,
            applied_guardrails: init.applied_guardrails,
            request_logging: init.request_logging,
            truncated_messages_for_logging: None,
            cost_breakdown: None,
            billed_token_rates: None,
            correlation: ownership,
            deferred: DeferredLogging::default(),
            callback_policy: init.callback_policy,
        })
    }
}

fn session_id_from_kwargs(kwargs: &IndexMap<String, Json>) -> Result<String, LoggingStateError> {
    match kwargs.get("litellm_session_id") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(session_id)) => Ok(session_id.clone()),
        Some(_) => Err(LoggingStateError::FieldType {
            field: "litellm_session_id",
        }),
    }
}

fn initial_model_call_details(init: &LoggingInit, trace_id: &str) -> IndexMap<String, Json> {
    let mut details = IndexMap::new();
    details.insert(
        "litellm_trace_id".to_owned(),
        Json::Text(trace_id.to_owned()),
    );
    details.insert(
        "litellm_call_id".to_owned(),
        Json::Text(init.litellm_call_id.clone()),
    );
    details.insert(
        "input".to_owned(),
        init.messages.as_ref().map_or(Json::Null, Json::from),
    );
    details.insert(
        "litellm_params".to_owned(),
        Json::Object(init.litellm_params.clone()),
    );
    details.insert(
        "applied_guardrails".to_owned(),
        applied_guardrails_json(init.applied_guardrails.as_deref()),
    );
    details.insert("model".to_owned(), Json::Text(init.model.clone()));
    details
}

fn applied_guardrails_json(guardrails: Option<&[String]>) -> Json {
    guardrails.map_or(Json::Null, |values| {
        Json::List(values.iter().cloned().map(Json::Text).collect())
    })
}
