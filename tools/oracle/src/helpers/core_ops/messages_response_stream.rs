mod state;
mod watch;

use std::collections::VecDeque;
use std::str::from_utf8;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_response_stream::source::{
    ProviderError, ProviderEvent, ProviderResponse, ResponsesEvent, ResponsesEventSource,
};
use betterlite_core::api::messages_response_stream::{
    MessagesResponseStream, MessagesResponseStreamInput, MessagesResponseStreamResult,
};
use betterlite_core::api::responses_stream_errors::failure::{FailureCause, FailureKind};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::runtime::Builder;
use uuid::Uuid;

use state::logging_state;
use watch::Captured;

type Stream = MessagesResponseStream<Scripted>;

const MESSAGE_ID: &str = "msg_00000000-0000-0000-0000-000000000007";

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Redaction {
    Enabled,
    Disabled,
}

// The two event shapes __anext__ can pull: a decoded mapping, or a provider object whose attributes getattr reads.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EventShape {
    Mapping,
    Attributes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventSpec {
    shape: EventShape,
    #[serde(default)]
    fields: Option<IndexMap<String, Json>>,
    #[serde(default)]
    attributes: Option<AttributesSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttributesSpec {
    #[serde(rename = "type", default)]
    event_type: Option<Json>,
    #[serde(default)]
    item: Option<Json>,
    #[serde(default)]
    item_id: Option<Json>,
    #[serde(default)]
    delta: Option<Json>,
    #[serde(default)]
    summary_index: Option<Json>,
    #[serde(default)]
    response: Option<ResponseSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseSpec {
    #[serde(default)]
    output: Vec<Json>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<ResponseApiUsage>,
    #[serde(default)]
    error: Option<ErrorSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorSpec {
    #[serde(default)]
    message: Option<Json>,
    #[serde(rename = "type", default)]
    error_type: Option<Json>,
    #[serde(default)]
    code: Option<Json>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Next,
    Drain,
    Sse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureSpec {
    text: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    status_code: Option<u16>,
}

impl FailureSpec {
    fn cause(&self) -> FailureCause {
        FailureCause {
            kind: FailureKind::Plain,
            text: self.text.clone(),
            message: self.message.clone().map(Json::Text),
            error_type: None,
            code: None,
            status_code: self
                .status_code
                .map(|status| Json::Number(serde_json::Number::from(status))),
            body: None,
            original_exception: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    events: Vec<EventSpec>,
    #[serde(default)]
    failure: Option<FailureSpec>,
    #[serde(default)]
    logging: bool,
    redaction: Redaction,
    steps: Vec<Step>,
}

// One SSE frame as the wire contract fixes it: the event name and the data line's decoded JSON.
#[derive(serde::Serialize)]
struct Frame {
    event: String,
    data: Json,
}

#[derive(serde::Serialize)]
struct Output {
    chunks: Vec<Json>,
    frames: Vec<Option<Frame>>,
    served_id_logged: Option<bool>,
    logs: Vec<String>,
}

struct Scripted {
    events: VecDeque<ResponsesEvent>,
    failure: Option<FailureCause>,
}

#[async_trait]
impl ResponsesEventSource for Scripted {
    async fn next_event(&mut self) -> Result<Option<ResponsesEvent>, FailureCause> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        self.failure.take().map_or_else(|| Ok(None), Err)
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

impl EventSpec {
    fn event(&self) -> ResponsesEvent {
        match self.shape {
            EventShape::Mapping => ResponsesEvent {
                attributes: ProviderEvent::default(),
                mapping: self.fields.clone(),
            },
            EventShape::Attributes => ResponsesEvent {
                attributes: self
                    .attributes
                    .as_ref()
                    .map_or_else(ProviderEvent::default, AttributesSpec::event),
                mapping: None,
            },
        }
    }
}

impl AttributesSpec {
    fn event(&self) -> ProviderEvent {
        ProviderEvent {
            event_type: self.event_type.clone(),
            item: self.item.clone(),
            item_id: self.item_id.clone(),
            delta: self.delta.clone(),
            summary_index: self.summary_index.clone(),
            response: self.response.as_ref().map(ResponseSpec::response),
        }
    }
}

impl ResponseSpec {
    fn response(&self) -> ProviderResponse {
        ProviderResponse {
            output: self.output.clone(),
            status: self.status.clone(),
            usage: self.usage.clone(),
            error: self.error.as_ref().map(ErrorSpec::error),
        }
    }
}

impl ErrorSpec {
    fn error(&self) -> ProviderError {
        ProviderError {
            message: self.message.clone(),
            error_type: self.error_type.clone(),
            code: self.code.clone(),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let (subscriber, captured) = watch::install();
        let outcome = runtime.block_on(drive(input, &captured));
        drop(subscriber);
        serde_json::to_value(outcome?).or_value_error()
    }
}

async fn drive(input: &Input, captured: &Captured) -> Result<Output, Box<HelperError>> {
    let mut logging = logging_state(input.logging, &input.model)?;
    let mut stream = MessagesResponseStream::new(
        Scripted {
            events: input.events.iter().map(EventSpec::event).collect(),
            failure: input.failure.as_ref().map(FailureSpec::cause),
        },
        &FixedIds,
        logging.as_mut(),
        MessagesResponseStreamInput {
            model: input.model.clone(),
            redactor: redactor_for(input.redaction)?,
            limits: ApiLimits::default(),
        },
    );
    let served_id_logged = logging
        .as_ref()
        .map(|state| state.streamed_anthropic_message_id.as_deref() == Some(MESSAGE_ID));
    let mut output = Output {
        chunks: Vec::new(),
        frames: Vec::new(),
        served_id_logged,
        logs: Vec::new(),
    };
    for step in &input.steps {
        match step {
            Step::Next => {
                let value = next_value(&mut stream).await?;
                output.chunks.push(value.unwrap_or(Json::Null));
            }
            Step::Drain => {
                drain(&mut stream, &mut output.chunks).await?;
                output.chunks.push(Json::Null);
            }
            Step::Sse => output.frames.push(next_frame(&mut stream).await?),
        }
    }
    output.logs = captured.expected(&input.model);
    Ok(output)
}

fn redactor_for(choice: Redaction) -> Result<Option<Arc<Redactor>>, Box<HelperError>> {
    match choice {
        Redaction::Enabled => Ok(Some(Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?,
        ))),
        Redaction::Disabled => Ok(None),
    }
}

async fn next_value(stream: &mut Stream) -> Result<Option<Json>, Box<HelperError>> {
    match stream.next_chunk().await.or_value_error()? {
        MessagesResponseStreamResult::Chunk(chunk) => Ok(Some(chunk)),
        MessagesResponseStreamResult::End => Ok(None),
    }
}

async fn drain(stream: &mut Stream, chunks: &mut Vec<Json>) -> Result<(), Box<HelperError>> {
    while let Some(chunk) = next_value(stream).await? {
        chunks.push(chunk);
    }
    Ok(())
}

async fn next_frame(stream: &mut Stream) -> Result<Option<Frame>, Box<HelperError>> {
    match stream.next_frame().await.or_value_error()? {
        Some(frame) => Ok(Some(frame_value(&frame)?)),
        None => Ok(None),
    }
}

// The frame's two lines: an "event: " name and a "data: " JSON line. Key order and escapes are no part of the wire contract.
fn frame_value(raw: &[u8]) -> Result<Frame, Box<HelperError>> {
    let text = from_utf8(raw).or_value_error()?;
    let body = text
        .strip_suffix("\n\n")
        .ok_or_else(|| invalid_frame(text))?;
    let mut lines = body.split('\n');
    let event = lines.next().and_then(|line| line.strip_prefix("event: "));
    let data = lines.next().and_then(|line| line.strip_prefix("data: "));
    let (Some(event), Some(data), None) = (event, data, lines.next()) else {
        return Err(invalid_frame(text));
    };
    let parsed = serde_json::from_str::<Json>(data).or_value_error()?;
    Ok(Frame {
        event: event.to_owned(),
        data: parsed,
    })
}

fn invalid_frame(text: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: format!("not an SSE frame: {text}"),
    })
}
