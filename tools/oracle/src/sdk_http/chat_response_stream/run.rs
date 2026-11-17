use std::collections::VecDeque;
use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream::driver::ChatResponseStream;
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
) -> Result<ChatResponseStream<Payloads, AssemblyContext>, SdkHttpError> {
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

fn step_sync(
    stream: &mut ChatResponseStream<Payloads, AssemblyContext>,
    step: ChatRunStep,
) -> Result<ChatResponseStreamStepOutcome, SdkHttpError> {
    let mut events = Vec::new();
    let mut end = false;
    loop {
        match stream
            .next_event()
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?
        {
            Some(event) => events.push(event_json(&event)?),
            None => {
                end = true;
                break;
            }
        }
        if step == ChatRunStep::Next {
            break;
        }
    }
    Ok(ChatResponseStreamStepOutcome { events, end })
}

async fn step_async(
    stream: &mut ChatResponseStream<Payloads, AssemblyContext>,
    step: ChatRunStep,
) -> Result<ChatResponseStreamStepOutcome, SdkHttpError> {
    let mut events = Vec::new();
    let mut end = false;
    loop {
        match stream
            .next_event_async()
            .await
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?
        {
            Some(event) => events.push(event_json(&event)?),
            None => {
                end = true;
                break;
            }
        }
        if step == ChatRunStep::Next {
            break;
        }
    }
    Ok(ChatResponseStreamStepOutcome { events, end })
}

fn event_json(event: &KnownResponseEvent) -> Result<serde_json::Value, SdkHttpError> {
    serde_json::to_value(event).map_err(SdkHttpError::Serialize)
}
