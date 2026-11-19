use betterlite_core::api::responses_stream_driver::errors::DriverError;
use betterlite_core::api::responses_stream_driver::live::{LiveStream, StreamEngine};
use betterlite_core::api::responses_stream_driver::source::PrefetchedSource;
use betterlite_core::api::responses_stream_driver::synthetic::{SyntheticSetup, SyntheticStream};
use betterlite_core::api::responses_stream_events::{
    BaseStreamStateInput, LoggingSnapshot, StreamTimes,
};
use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::ports::Dispatch;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use chrono::DateTime;
use indexmap::IndexMap;

use super::assemble::{Decoder, Environment, Event, NoApiBase, NoCost, NoTokenization};
use super::call::{Call, Mode, RunOutcome, RunSpec, Step, StepOutcome};
use super::source::{self, Opened};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;

mod outcome;

use self::outcome::{failure_of, view};

enum Stream<'stream> {
    Live(Box<LiveStream<'stream, Decoder, NoTokenization, NoCost>>),
    Synthetic(Box<SyntheticStream<'stream>>),
}
impl Stream<'_> {
    async fn next_async(&mut self) -> Result<Option<Event>, DriverError> {
        match self {
            Self::Live(stream) => stream.next_event().await,
            Self::Synthetic(stream) => stream.advance(Dispatch::Spawned).await,
        }
    }
    fn next_blocking(&mut self) -> Result<Option<Event>, DriverError> {
        match self {
            Self::Live(stream) => futures_executor::block_on(stream.drive(Dispatch::Inline)),
            Self::Synthetic(stream) => futures_executor::block_on(stream.advance(Dispatch::Inline)),
        }
    }
}
fn live_stream<'stream>(
    hooks: &'stream ResponsesStreamHooks<'stream>,
    environment: &'stream Environment,
    headers: &'stream IndexMap<String, String>,
    call: &Call,
    items: Opened,
) -> Stream<'stream> {
    let state = BaseStreamStateInput {
        model: &call.model,
        custom_llm_provider: Some("openai"),
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: LoggingSnapshot::read(&environment.logging),
        headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    };
    Stream::Live(Box::new(LiveStream::new(
        StreamEngine {
            transform: Some(&Decoder),
            tokenization: None::<&NoTokenization>,
            calculator: None::<&NoCost>,
            hooks,
            logging: &environment.logging,
            details: IndexMap::new(),
            limits: &environment.limits,
        },
        state,
        Box::new(PrefetchedSource::new(items)),
    )))
}
fn synthesize<'stream>(
    hooks: &'stream ResponsesStreamHooks<'stream>,
    environment: &'stream Environment,
    headers: &'stream IndexMap<String, String>,
    spec: &RunSpec,
    model: &'stream mut String,
) -> Result<Stream<'stream>, SdkHttpError> {
    let cached = matches!(spec.mode, Mode::Cached);
    let response = spec
        .response
        .as_ref()
        .ok_or_else(|| SdkHttpError::Invalid("a fake or cached run needs a response".to_owned()))?;
    let value = serde_json::to_value(response).map_err(SdkHttpError::Serialize)?;
    let response: ResponsesApiResponse =
        serde_json::from_value(value).map_err(SdkHttpError::ResultParse)?;
    if cached {
        *model = response.model.clone().unwrap_or_default();
    }
    let state = BaseStreamStateInput {
        model: model.as_str(),
        custom_llm_provider: Some(if cached { "cached_response" } else { "openai" }),
        litellm_metadata: None,
        request_data: None,
        call_type: None,
        logging: LoggingSnapshot::read(&environment.logging),
        headers,
        api_base: &NoApiBase,
        times: times(),
        max_streaming_duration_seconds: None,
    };
    let setup = SyntheticSetup {
        state,
        hooks,
        cache_hit: cached.then_some(true),
        persist_before_logging: !cached,
    };
    let synthesized = SyntheticStream::build(response, setup, None::<&NoCost>)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(Stream::Synthetic(Box::new(synthesized)))
}
pub(super) async fn run(
    client: &ProxiedClient,
    call: &Call,
    spec: &RunSpec,
    api_base: &str,
) -> Result<RunOutcome, SdkHttpError> {
    let environment = Environment::new()?;
    let hooks = environment.hooks();
    let headers = IndexMap::new();
    let opened = match spec.mode {
        Mode::Live => Some(source::prefetch(client, call, api_base).await?),
        Mode::Scripted => Some(source::scripted(spec)),
        Mode::Fake | Mode::Cached => None,
    };
    let mut model = call.model.clone();
    let mut stream = match opened {
        Some(items) => live_stream(&hooks, &environment, &headers, call, items),
        None => synthesize(&hooks, &environment, &headers, spec, &mut model)?,
    };
    let (steps, failure) = drive_stream(&mut stream, spec).await?;
    match &mut stream {
        Stream::Live(live) => live.drain_pending_logging().await,
        Stream::Synthetic(synthetic) => synthetic.drain_pending_logging().await,
    }
    let (effects, logged) = environment.recorder.take();
    Ok(RunOutcome {
        steps,
        failure,
        effects,
        logged,
    })
}

async fn drive_stream(
    stream: &mut Stream<'_>,
    spec: &RunSpec,
) -> Result<(Vec<StepOutcome>, Option<Failure>), SdkHttpError> {
    let mut steps = Vec::new();
    for &step in &spec.steps {
        let (events, end, failure) = drive_step(stream, spec.asynchronous, step).await?;
        steps.push(StepOutcome { events, end });
        if failure.is_some() {
            return Ok((steps, failure));
        }
    }
    Ok((steps, None))
}
async fn drive_step(
    stream: &mut Stream<'_>,
    asynchronous: bool,
    step: Step,
) -> Result<(Vec<Json>, bool, Option<Failure>), SdkHttpError> {
    let mut events = Vec::new();
    loop {
        let outcome = if asynchronous {
            stream.next_async().await
        } else {
            stream.next_blocking()
        };
        match outcome {
            Ok(Some(event)) => events.push(view(&event)?),
            Ok(None) => return Ok((events, true, None)),
            Err(error) => return Ok((events, false, Some(failure_of(&error)?))),
        }
        if matches!(step, Step::Next) {
            return Ok((events, false, None));
        }
    }
}
fn times() -> StreamTimes {
    StreamTimes {
        now: DateTime::UNIX_EPOCH.fixed_offset(),
        created_timestamp: 0.0,
    }
}
