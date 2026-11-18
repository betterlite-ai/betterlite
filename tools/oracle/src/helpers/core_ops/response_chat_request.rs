use std::time::Duration;

use betterlite_core::api::response_chat_history::history::HistoryMessage;
use betterlite_core::api::response_chat_request::records::ChatRequestInput;
use betterlite_core::api::response_chat_request::request::build_request;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use diagnostics::Diagnostics;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod diagnostics;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    messages: Vec<HistoryMessage>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Value>,
    headers: IndexMap<String, String>,
    reasoning_auto_summary: bool,
    client: Option<String>,
}

// the flat request LiteLLM returns, so the oracle compares one object on both sides
#[derive(Serialize)]
struct RequestView<'request> {
    #[serde(flatten)]
    params: &'request ResponsesApiRequestParams,
    #[serde(flatten)]
    litellm_params: &'request IndexMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_headers: Option<&'request IndexMap<String, String>>,
    client: Option<&'request str>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut optional_params = input.optional_params.clone();
        let mut logging = logging_state()?;
        let diagnostics = Diagnostics::default();
        let subscriber = tracing_subscriber::registry().with(diagnostics.clone());
        subscriber::with_default(subscriber, || {
            let request = build_request(
                ChatRequestInput {
                    model: &input.model,
                    messages: &input.messages,
                    optional_params: &mut optional_params,
                    litellm_params: &input.litellm_params,
                    headers: &input.headers,
                    reasoning_auto_summary: input.reasoning_auto_summary,
                },
                &mut logging,
                input.client.as_deref(),
            )
            .or_value_error()?;
            let view = RequestView {
                params: &request.params,
                litellm_params: &request.litellm_params,
                extra_headers: request.extra_headers.as_ref(),
                client: request.client,
            };
            Ok(json!({
                "request": view,
                "optional_params": optional_params,
                "call_type": request.logging.call_type.clone(),
                "diagnostics": diagnostics.drain(),
            }))
        })
    }
}

fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
            call_type: "completion".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .or_value_error()?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .or_value_error()
}
