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
use indexmap::IndexMap;

use super::assembly::AssemblyContext;
use super::call::{ChatResponseStreamCall, ChatRunMode, ChatRunSpec, ChatRunStep};
use super::source::PayloadSource;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::native;
use crate::sdk_http::result::{
    ChatResponseStreamRunOutcome, ChatResponseStreamStepOutcome, Failure,
};
use crate::sdk_http::wrapper_stream::{self, Prepared};

const NORMALIZED_ID: &str = "00000000-0000-0000-0000-000000000000";

type ChatStream = ChatResponseStream<PayloadSource, AssemblyContext>;
type Step = ChatResponseStreamStepOutcome;
type StepResult = Result<Step, SdkHttpError>;
type EventResult = Result<Option<KnownResponseEvent>, ChatResponseStreamError>;

// What one run's POST left: the SSE payloads to drive, or the failure the provider raised before any event
pub(super) type Opened = Result<VecDeque<Vec<u8>>, Failure>;

pub(super) async fn open(
    prepared: &Prepared,
    call: &ChatResponseStreamCall,
) -> Result<Opened, SdkHttpError> {
    let request = wrapper_stream::wire_request(call)?;
    match wrapper_stream::open(prepared, &request).await {
        Ok(response) => Ok(Ok(wrapper_stream::payloads(response).await?)),
        Err(ChatError::Provider { response }) => Ok(Err(native::failure(&call.model, &response)?)),
        Err(other) => Err(SdkHttpError::Chat(Box::new(other))),
    }
}

pub(super) async fn run(
    call: &ChatResponseStreamCall,
    spec: &ChatRunSpec,
    opened: Opened,
) -> Result<ChatResponseStreamRunOutcome, SdkHttpError> {
    let payloads = match opened {
        Ok(payloads) => payloads,
        Err(failure) => {
            return Ok(ChatResponseStreamRunOutcome {
                steps: Vec::new(),
                failure: Some(failure),
            });
        }
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
    let input = Json::Text(call.request_input.clone());
    let params = call
        .responses_api_request
        .clone()
        .unwrap_or_else(|| Json::Object(IndexMap::new()));
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
        EventSources {
            clock: Arc::new(|| Ok(0)),
            ids: Arc::new(|| NORMALIZED_ID.to_owned()),
        },
        PayloadSource::new(call.model.clone(), payloads),
    )
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

// The events one step stacks up, and whether the provider ended the stream under it
fn push(outcome: &mut Step, next: EventResult) -> Result<(), SdkHttpError> {
    match next {
        Ok(Some(event)) => {
            let encoded = serde_json::to_value(&event).map_err(SdkHttpError::Serialize)?;
            outcome.events.push(encoded);
        }
        Ok(None) => outcome.end = true,
        Err(error) => return Err(SdkHttpError::Invalid(error.to_string())),
    }
    Ok(())
}

fn step_sync(stream: &mut ChatStream, step: ChatRunStep) -> StepResult {
    let mut outcome = Step::default();
    while !outcome.end && (step != ChatRunStep::Next || outcome.events.is_empty()) {
        push(&mut outcome, stream.next_event())?;
    }
    Ok(outcome)
}

async fn step_async(stream: &mut ChatStream, step: ChatRunStep) -> StepResult {
    let mut outcome = Step::default();
    while !outcome.end && (step != ChatRunStep::Next || outcome.events.is_empty()) {
        push(&mut outcome, stream.next_event_async().await)?;
    }
    Ok(outcome)
}
