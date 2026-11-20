mod state;
mod watch;

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_response_stream::source::ResponsesEventSource;
use betterlite_core::api::messages_response_stream::{
    MessagesResponseStream, MessagesResponseStreamInput, MessagesResponseStreamResult,
};
use betterlite_core::api::responses_stream_errors::failure::{FailureCause, FailureKind};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
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
    events: Vec<Json>,
    #[serde(default)]
    failure: Option<FailureSpec>,
    #[serde(default)]
    logging: bool,
    redaction: Redaction,
    steps: Vec<Step>,
}

#[derive(serde::Serialize)]
struct Output {
    chunks: Vec<Json>,
    frames: Vec<Option<String>>,
    served_id_logged: Option<bool>,
    logs: Vec<String>,
}

struct Scripted {
    events: VecDeque<Json>,
    failure: Option<FailureCause>,
}

#[async_trait]
impl ResponsesEventSource for Scripted {
    async fn next_event(&mut self) -> Result<Option<Json>, FailureCause> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        match self.failure.take() {
            Some(failure) => Err(failure),
            None => Ok(None),
        }
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
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
            events: VecDeque::from(input.events.clone()),
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

async fn next_frame(stream: &mut Stream) -> Result<Option<String>, Box<HelperError>> {
    match stream.next_frame().await.or_value_error()? {
        Some(frame) => Ok(Some(String::from_utf8(frame).or_value_error()?)),
        None => Ok(None),
    }
}
