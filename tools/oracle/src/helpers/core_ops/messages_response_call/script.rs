use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_core::api::messages_response_call::error::MessagesResponseCallError;
use betterlite_core::api::messages_response_call::records::{
    MessagesResponseCallAnswer, MessagesResponseCallInput, MessagesResponseCallResult,
    ResponsesAnswer,
};
use betterlite_core::api::messages_response_call::seams::ResponsesCreatePort;
use betterlite_core::api::messages_response_call::service::messages_response_call;
use betterlite_core::api::messages_response_stream::source::{
    ResponsesEvent, ResponsesEventSource,
};
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_create::error::CreateFailure;
use betterlite_core::api::responses_create_http::service::LogContext;
use betterlite_core::api::responses_stream_errors::failure::FailureCause;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_oracle_helper_api::error::{ErrorCode, HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use super::events::EventSpec;
use super::fixture::Scripts;
use super::view::{CallView, Output, frame_value};

// The production call plus the scripted seams: the provider capability, the metadata lookups and what litellm.aresponses answers.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    call: MessagesResponseCallInput,
    include_supported: bool,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model_info: Option<IndexMap<String, Json>>,
    answer: AnswerSpec,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
enum AnswerSpec {
    Response { body: Json },
    Stream { events: Vec<EventSpec> },
    Other { kind: String },
}

impl AnswerSpec {
    fn plan(&self) -> Result<AnswerPlan, Box<HelperError>> {
        match self {
            Self::Response { body } => Ok(AnswerPlan::Response(Box::new(
                transcode::<Json, ResponsesApiResponse>(body).or_value_error()?,
            ))),
            Self::Stream { events } => Ok(AnswerPlan::Stream(
                events.iter().map(EventSpec::event).collect(),
            )),
            Self::Other { kind } => Ok(AnswerPlan::Other(kind.clone())),
        }
    }
}

enum AnswerPlan {
    Response(Box<ResponsesApiResponse>),
    Stream(VecDeque<ResponsesEvent>),
    Other(String),
}

struct ScriptedPort {
    answer: Mutex<Option<AnswerPlan>>,
}

struct ScriptedEvents {
    events: VecDeque<ResponsesEvent>,
}

#[async_trait]
impl ResponsesEventSource for ScriptedEvents {
    async fn next_event(&mut self) -> Result<Option<ResponsesEvent>, FailureCause> {
        Ok(self.events.pop_front())
    }
}

#[async_trait]
impl ResponsesCreatePort for ScriptedPort {
    type Stream = ScriptedEvents;

    async fn create(
        &self,
        _call: &ResponsesCreateCall,
        _log: &mut LogContext<'_, '_>,
    ) -> Result<ResponsesAnswer<Self::Stream>, CreateFailure> {
        let plan = match self.answer.lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        match plan {
            Some(AnswerPlan::Response(response)) => Ok(ResponsesAnswer::Response(response)),
            Some(AnswerPlan::Stream(events)) => {
                Ok(ResponsesAnswer::Stream(ScriptedEvents { events }))
            }
            Some(AnswerPlan::Other(kind)) => Ok(ResponsesAnswer::Other { kind }),
            None => Err(unscripted()),
        }
    }
}

fn unscripted() -> CreateFailure {
    CreateFailure {
        class: "APIError",
        status: 500,
        kind: None,
        param: None,
        code: None,
        message: String::from("the scripted port ran twice"),
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) async fn drive(input: &Input) -> Result<Output, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let sink = Sink;
    let pre_post = environment(&redactor, &sink)?;
    let mut state = logging_state(&redactor)?;
    let scripts = Scripts::new(
        input.include_supported,
        input.provider.clone(),
        input.model_info.clone(),
    )?;
    let port = ScriptedPort {
        answer: Mutex::new(Some(input.answer.plan()?)),
    };
    let outcome = {
        let mut log = LogContext {
            state: &mut state,
            env: &pre_post,
        };
        messages_response_call(&input.call, &scripts.seams(), &port, &FixedIds, &mut log).await
    };
    finish(input, outcome, &redactor).await
}

fn environment<'env>(
    redactor: &'env Redactor,
    log: &'env Sink,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").or_value_error()?,
    })
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(serde_json::json!({
        "model": "gpt-4o",
        "call_type": "responses",
        "start_time": "2026-01-01T00:00:00+00:00",
        "litellm_call_id": "call-1",
        "function_id": "fn-1",
    }))
    .or_value_error()?;
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .or_value_error()
}

async fn finish(
    input: &Input,
    outcome: Result<MessagesResponseCallResult<ScriptedEvents>, MessagesResponseCallError>,
    redactor: &Redactor,
) -> Result<Output, Box<HelperError>> {
    let result = match outcome {
        Ok(result) => result,
        Err(error) => {
            let failure =
                error.normalize(&input.call.request.model, &provider_label(input), redactor);
            return Err(helper_error(&failure));
        }
    };
    let view = CallView::new(&result.call);
    let request = transcode::<CallView<'_>, Json>(&view).or_value_error()?;
    match result.answer {
        MessagesResponseCallAnswer::Native(message) => Ok(Output {
            request,
            native: Some(*message),
            frames: None,
        }),
        MessagesResponseCallAnswer::Stream(mut stream) => {
            let mut frames = Vec::new();
            while let Some(frame) = stream.next_frame().await.or_value_error()? {
                frames.push(frame_value(&frame)?);
            }
            Ok(Output {
                request,
                native: None,
                frames: Some(frames),
            })
        }
    }
}

// The provider exception_type would name: the forwarded hint, else the provider the seam resolved.
fn provider_label(input: &Input) -> String {
    match input.call.forwarded.get("custom_llm_provider") {
        Some(JsonValue::Text(provider)) => provider.clone(),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => input.provider.clone().unwrap_or_default(),
    }
}

fn helper_error(failure: &CreateFailure) -> Box<HelperError> {
    Box::new(HelperError {
        class: failure.class.to_owned(),
        status: Some(failure.status),
        error_type: Some(failure.kind.clone()),
        param: Some(failure.param.clone()),
        code: failure.code.clone().map(ErrorCode::Text).map(Some),
        message: failure.message.clone(),
    })
}
