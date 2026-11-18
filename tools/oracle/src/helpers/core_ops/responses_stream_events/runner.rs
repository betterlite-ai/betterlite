use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_stream_events::{
    ProcessEventError, ResponsesEventProcessor, ResponsesStreamBaseState,
};
use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::failure::await_pending_logging;
use betterlite_core::api::responses_stream_hooks::ports::Dispatch;
use betterlite_core::api::responses_stream_hooks::state::ResponsesStreamHookState;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::support::{
    Calculator, FixedTokens, Probe, ProviderTransform, TransformFailure, hooks, logging_object,
    state,
};
use super::{EstimateMode, Input, TransformMode};

pub(super) async fn run(input: &Input) -> HelperResult {
    let clock = DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").or_value_error()?;
    let probe = Arc::new(Probe::new(input, clock));
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mut rig = Rig::new(input, &probe, &redactor)?;
    rig.drive(&input.payloads).await?;
    rig.finish().await;
    rig.projection()
}

struct Rig<'context> {
    hooks: ResponsesStreamHooks<'context>,
    logging: StreamLoggingObject,
    probe: Arc<Probe>,
    transform: Option<ProviderTransform>,
    tokenization: Option<FixedTokens>,
    calculator: Calculator,
    limits: ApiLimits,
    details: IndexMap<String, Json>,
    base: ResponsesStreamBaseState,
    callback: ResponsesStreamHookState,
    events: Vec<Value>,
    error: Option<String>,
}

impl<'context> Rig<'context> {
    fn new(
        input: &Input,
        probe: &Arc<Probe>,
        redactor: &'context Redactor,
    ) -> Result<Self, Box<HelperError>> {
        let transform = match input.transform {
            TransformMode::Configured => Some(ProviderTransform::new(Arc::clone(probe))),
            TransformMode::Absent => None,
        };
        let tokenization = match input.estimate {
            EstimateMode::Fixed => Some(FixedTokens::new(Arc::clone(probe))),
            EstimateMode::Absent => None,
        };
        Ok(Self {
            hooks: hooks(probe, redactor),
            logging: logging_object(redactor).or_value_error()?,
            probe: Arc::clone(probe),
            transform,
            tokenization,
            calculator: Calculator::new(Arc::clone(probe)),
            limits: ApiLimits::default(),
            details: IndexMap::new(),
            base: state(input, probe),
            callback: ResponsesStreamHookState::default(),
            events: Vec::new(),
            error: None,
        })
    }

    async fn drive(&mut self, payloads: &[String]) -> HelperResult {
        for payload in payloads {
            match self.payload(payload).await {
                Ok(Some(event)) => self.events.push(serde_json::to_value(event)?),
                Ok(None) => self.events.push(Value::Null),
                Err(failure) => {
                    self.error = Some(failure.to_string());
                    break;
                }
            }
        }
        Ok(())
    }

    async fn payload(
        &mut self,
        payload: &str,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, ProcessEventError<TransformFailure>> {
        let mut processor = ResponsesEventProcessor {
            transform: self.transform.as_ref(),
            tokenization: self.tokenization.as_ref(),
            calculator: Some(&self.calculator),
            hooks: &self.hooks,
            details: &mut self.details,
            dispatch: Dispatch::Inline,
            limits: &self.limits,
        };
        processor
            .process(payload, &mut self.base, &mut self.callback, &self.logging)
            .await
    }

    async fn finish(&mut self) {
        await_pending_logging(&mut self.callback).await;
        self.base.pending_tasks.close();
        self.base.pending_tasks.wait().await;
    }

    fn projection(&self) -> HelperResult {
        let completion_start_set = self
            .logging
            .read(&mut |state| state.completion_start_time)
            .flatten()
            .is_some();
        let completed = match self.base.completed_response.as_ref() {
            Some(event) => serde_json::to_value(event)?,
            None => Value::Null,
        };
        let projection = json!({
            "events": self.events,
            "error": self.error,
            "state": {
                "finished": self.base.finished,
                "failure_handled": self.base.failure_handled,
                "logged": self.base.completed_response_logged,
                "cached": self.base.completed_response_cached,
                "completion_start_set": completion_start_set,
                "completed": completed,
                "generated_content": self.base.generated_content,
                "generated_tool_arguments": self.base.generated_tool_arguments,
            },
            "records": self.probe.records(),
        });
        Ok(normalize(projection))
    }
}

fn normalize(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_, child)| !child.is_null())
                .map(|(name, child)| (name, normalize(child)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        scalar @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => scalar,
    }
}
