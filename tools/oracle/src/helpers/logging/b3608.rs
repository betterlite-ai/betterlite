use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::build::StreamLoggingInput;
// ast-grep-ignore: no-long-names
use betterlite_logging::passthrough_stream_logging::build::build_passthrough_logging_result as build_stream_result;
use betterlite_logging::passthrough_stream_logging::lines::convert_raw_bytes_to_str_lines;
// ast-grep-ignore: no-long-names
use betterlite_logging::passthrough_stream_logging::model::extract_model_for_cost_injection as model_for_cost;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, EndpointType, HandlerError, HandlerOutcome,
    ProviderChunkHandlers,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Lines { chunks } => Ok(json!({
                "lines": convert_raw_bytes_to_str_lines(chunks),
            })),
            Input::ExtractModel {
                request_body,
                model_call_details_model,
                url_route,
                endpoint_type,
            } => {
                let body = body_json(request_body.as_ref())?;
                let model = model_for_cost(
                    body.as_ref(),
                    model_call_details_model.as_deref(),
                    url_route,
                    EndpointType::from(*endpoint_type),
                );
                Ok(json!({"model": model}))
            }
            Input::BuildResult {
                endpoint_type,
                url_route,
                request_body,
                model,
                chunks,
            } => {
                let body = body_json(request_body.as_ref())?;
                build_result(*endpoint_type, url_route, body, model.clone(), chunks)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Lines {
        chunks: Vec<Vec<u8>>,
    },
    ExtractModel {
        #[serde(default)]
        request_body: Option<Value>,
        #[serde(default)]
        model_call_details_model: Option<String>,
        url_route: String,
        endpoint_type: EndpointName,
    },
    BuildResult {
        endpoint_type: EndpointName,
        url_route: String,
        #[serde(default)]
        request_body: Option<Value>,
        #[serde(default)]
        model: Option<String>,
        chunks: Vec<Vec<u8>>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndpointName {
    #[serde(rename = "vertex-ai")]
    VertexAi,
    Gemini,
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    Tinyfish,
    Generic,
    Decisions,
}

impl From<EndpointName> for EndpointType {
    fn from(name: EndpointName) -> Self {
        match name {
            EndpointName::VertexAi => Self::VertexAi,
            EndpointName::Gemini => Self::Gemini,
            EndpointName::Anthropic => Self::Anthropic,
            EndpointName::OpenAi => Self::OpenAi,
            EndpointName::Tinyfish => Self::Tinyfish,
            EndpointName::Generic => Self::Generic,
            EndpointName::Decisions => Self::Decisions,
        }
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

struct NoHandler;

impl CollectedChunksHandler for NoHandler {
    fn handle(&self, _call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        Ok(HandlerOutcome::default())
    }
}

fn build_result(
    endpoint: EndpointName,
    url_route: &str,
    request_body: Option<Json>,
    model: Option<String>,
    chunks: &[Vec<u8>],
) -> crate::helpers::HelperResult {
    let logging = logging_state()?;
    let object = Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoDispatch),
    });
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(NoHandler);
    let input = StreamLoggingInput {
        logging_obj: object,
        passthrough: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
        providers: Arc::new(ProviderChunkHandlers {
            anthropic: Arc::clone(&handler),
            vertex: Arc::clone(&handler),
            gemini: Arc::clone(&handler),
            openai: handler,
        }),
        url_route: url_route.to_owned(),
        request_body,
        endpoint_type: EndpointType::from(endpoint),
        start_time: fixed_time()?,
        end_time: fixed_time()?,
        raw_bytes: chunks.to_vec(),
        model,
    };
    let output = build_stream_result(&input).map_err(|error| invalid(&error.to_string()))?;
    Ok(json!({
        "result": output.result.into_json(),
        "kwargs": output.kwargs.entries,
    }))
}

fn logging_state() -> Result<LoggingState, Box<crate::helpers::HelperError>> {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: fixed_time()?,
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(true),
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
    };
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let redactor = Redactor::new(16).map_err(|error| invalid(&error.to_string()))?;
    LoggingState::new(
        init,
        setup,
        &redactor,
        || "trace-1".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|error| invalid(&error.to_string()))
}

fn fixed_time() -> Result<DateTime<FixedOffset>, Box<crate::helpers::HelperError>> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
        .map_err(|error| invalid(&error.to_string()))
}

fn body_json(
    request_body: Option<&Value>,
) -> Result<Option<Json>, Box<crate::helpers::HelperError>> {
    request_body
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))
        })
        .transpose()
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
