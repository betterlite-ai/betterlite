use std::collections::VecDeque;
use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream::driver::ChatResponseStream;
use betterlite_core::api::chat_response_stream::error::ChatResponseStreamError;
use betterlite_core::api::chat_response_stream_items::sources::EventSources;
use betterlite_core::api::chat_response_stream_tools::state::StreamSetup;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::chat::execution::ChatError;

use super::assembly::AssemblyContext;
use super::call::{ChatResponseStreamCall, ChatRunMode, ChatRunSpec, ChatRunStep};
use super::source::Payloads;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::native;
use crate::sdk_http::result::{
    ChatResponseStreamRunOutcome, ChatResponseStreamStepOutcome, Failure,
};
use crate::sdk_http::wrapper_stream::{self, Prepared};

const NORMALIZED_ID: &str = "00000000-0000-0000-0000-000000000000";

type ChatStream = ChatResponseStream<Payloads, AssemblyContext>;
type StepResult = Result<ChatResponseStreamStepOutcome, SdkHttpError>;
type EventResult = Result<Option<KnownResponseEvent>, ChatResponseStreamError>;

// What one run's POST left: the SSE payloads to drive, or the failure the provider raised before any event
pub(super) enum Opened {
    Payloads(VecDeque<Vec<u8>>),
    Failure(Failure),
}

pub(super) async fn open(
    prepared: &Prepared,
    call: &ChatResponseStreamCall,
) -> Result<Opened, SdkHttpError> {
    let request = wrapper_stream::wire_request(call)?;
    match wrapper_stream::open(prepared, &request).await {
        Ok(response) => Ok(Opened::Payloads(wrapper_stream::payloads(response).await?)),
        Err(ChatError::Provider { response }) => {
            Ok(Opened::Failure(native::failure(&call.model, &response)?))
        }
        Err(other) => Err(SdkHttpError::Chat(Box::new(other))),
    }
}

// One case run: the chosen iterator over one mock stream, driven step by step
pub(super) async fn run(
    call: &ChatResponseStreamCall,
    spec: &ChatRunSpec,
    opened: Opened,
) -> Result<ChatResponseStreamRunOutcome, SdkHttpError> {
    let payloads = match opened {
        Opened::Failure(failure) => {
            return Ok(ChatResponseStreamRunOutcome {
                steps: Vec::new(),
                failure: Some(failure),
            });
        }
        Opened::Payloads(payloads) => payloads,
    };
    let mut stream = build(call, payloads)?;
    let mut steps = Vec::with_capacity(spec.steps.len());
    for step in &spec.steps {
        steps.push(match spec.mode {
            ChatRunMode::Sync => step_sync(&mut stream, *step)?,
            ChatRunMode::Async => step_async(&mut stream, *step).await?,
        });
    }
    Ok(ChatResponseStreamRunOutcome {
        steps,
        failure: None,
    })
}

fn build(
    call: &ChatResponseStreamCall,
    payloads: VecDeque<Vec<u8>>,
) -> Result<ChatStream, SdkHttpError> {
    let params = call.params_json()?;
    let input = Json::Text(call.request_input.clone());
    ChatResponseStream::new(
        StreamSetup {
            model: &call.model,
            logging_context: AssemblyContext::new()?,
            request_input: &input,
            responses_api_request: &params,
            custom_llm_provider: Some("openai"),
            litellm_metadata: None,
            limits: ApiLimits::default(),
        },
        sources(),
        Payloads::new(call.model.clone(), payloads),
    )
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

// The injected ids and time a case may not depend on: every generated id and timestamp is fixed here
fn sources() -> EventSources {
    EventSources {
        clock: Arc::new(|| Ok(0)),
        ids: Arc::new(|| NORMALIZED_ID.to_owned()),
    }
}

// The events one step stacks up, and whether the provider ended the stream under it
#[derive(Default)]
struct StepEvents {
    events: Vec<serde_json::Value>,
    end: bool,
}

impl StepEvents {
    fn push(&mut self, next: EventResult) -> Result<(), SdkHttpError> {
        match next {
            Ok(Some(event)) => {
                let value = serde_json::to_value(&event).map_err(SdkHttpError::Serialize)?;
                self.events.push(value);
            }
            Ok(None) => self.end = true,
            Err(error) => return Err(SdkHttpError::Invalid(error.to_string())),
        }
        Ok(())
    }
}

impl From<StepEvents> for ChatResponseStreamStepOutcome {
    fn from(events: StepEvents) -> Self {
        Self {
            events: events.events,
            end: events.end,
        }
    }
}

fn step_sync(stream: &mut ChatStream, step: ChatRunStep) -> StepResult {
    let mut outcome = StepEvents::default();
    while !outcome.end && (step != ChatRunStep::Next || outcome.events.is_empty()) {
        outcome.push(stream.next_event())?;
    }
    Ok(ChatResponseStreamStepOutcome::from(outcome))
}

async fn step_async(stream: &mut ChatStream, step: ChatRunStep) -> StepResult {
    let mut outcome = StepEvents::default();
    while !outcome.end && (step != ChatRunStep::Next || outcome.events.is_empty()) {
        outcome.push(stream.next_event_async().await)?;
    }
    Ok(ChatResponseStreamStepOutcome::from(outcome))
}
