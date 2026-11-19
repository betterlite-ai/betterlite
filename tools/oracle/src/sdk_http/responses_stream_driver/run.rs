use betterlite_core::api::responses_stream_driver::errors::DriverError;
use betterlite_core::api::responses_stream_driver::live::{
    LiveStream, LiveStreamSetup, StreamEngine,
};
use betterlite_core::api::responses_stream_driver::source::PrefetchedSource;
use betterlite_core::api::responses_stream_driver::synthetic::{SyntheticSetup, SyntheticStream};
use betterlite_core::api::responses_stream_events::StreamTimes;
use betterlite_http::proxy::ProxiedClient;
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
    Synthetic(SyntheticStream<'stream>),
}

impl Stream<'_> {
    async fn next_async(&mut self) -> Result<Option<Event>, DriverError> {
        match self {
            Self::Live(stream) => stream.next_event().await,
            Self::Synthetic(stream) => stream.next_event_async().await,
        }
    }

    fn next_blocking(&mut self) -> Result<Option<Event>, DriverError> {
        match self {
            Self::Live(stream) => stream.next_event_blocking(),
            Self::Synthetic(stream) => stream.next_event(),
        }
    }

    async fn drain_logging(&mut self) {
        match self {
            Self::Live(stream) => stream.drain_logging().await,
            Self::Synthetic(stream) => stream.drain_logging().await,
        }
    }
}

pub(super) async fn run(
    client: &ProxiedClient,
    call: &Call,
    spec: &RunSpec,
    api_base: &str,
) -> Result<RunOutcome, SdkHttpError> {
    match spec.mode {
        Mode::LiveSync => drive_live(client, call, spec, api_base, false).await,
        Mode::LiveAsync => drive_live(client, call, spec, api_base, true).await,
        Mode::ScriptedSync => drive_scripted(call, spec, false).await,
        Mode::ScriptedAsync => drive_scripted(call, spec, true).await,
        Mode::FakeSync => drive_fake(call, spec, false).await,
        Mode::FakeAsync => drive_fake(call, spec, true).await,
        Mode::CachedSync => drive_cached(call, spec, false).await,
        Mode::CachedAsync => drive_cached(call, spec, true).await,
    }
}

async fn drive_live(
    client: &ProxiedClient,
    call: &Call,
    spec: &RunSpec,
    api_base: &str,
    asynchronous: bool,
) -> Result<RunOutcome, SdkHttpError> {
    let items = source::prefetch(client, call, api_base).await?;
    drive_payloads(call, spec, items, asynchronous).await
}

async fn drive_scripted(
    call: &Call,
    spec: &RunSpec,
    asynchronous: bool,
) -> Result<RunOutcome, SdkHttpError> {
    drive_payloads(call, spec, source::scripted(spec), asynchronous).await
}

async fn drive_payloads(
    call: &Call,
    spec: &RunSpec,
    items: Opened,
    asynchronous: bool,
) -> Result<RunOutcome, SdkHttpError> {
    let environment = Environment::new()?;
    let hooks = environment.hooks();
    let transform = Decoder;
    let api_base = NoApiBase;
    let headers = IndexMap::new();
    let mut stream = Stream::Live(Box::new(LiveStream::new(
        StreamEngine {
            transform: Some(&transform),
            tokenization: None::<&NoTokenization>,
            calculator: None::<&NoCost>,
            hooks: &hooks,
            logging: &environment.logging,
            details: IndexMap::new(),
            limits: &environment.limits,
        },
        LiveStreamSetup {
            model: &call.model,
            custom_llm_provider: Some("openai"),
            litellm_metadata: None,
            request_data: None,
            call_type: None,
            headers: &headers,
            api_base: &api_base,
            times: times(),
            max_streaming_duration_seconds: None,
        },
        Box::new(PrefetchedSource::new(items)),
    )));
    finish(&mut stream, spec, asynchronous, &environment).await
}

async fn drive_fake(
    call: &Call,
    spec: &RunSpec,
    asynchronous: bool,
) -> Result<RunOutcome, SdkHttpError> {
    let environment = Environment::new()?;
    let hooks = environment.hooks();
    let api_base = NoApiBase;
    let headers = IndexMap::new();
    let response = response_of(spec)?;
    let synthetic = SyntheticStream::build::<NoCost>(
        response,
        SyntheticSetup {
            hooks: &hooks,
            model: &call.model,
            custom_llm_provider: Some("openai"),
            litellm_metadata: None,
            request_data: None,
            call_type: None,
            headers: &headers,
            api_base: &api_base,
            logging: &environment.logging,
            times: times(),
            cache_hit: None,
            persist_before_logging: true,
        },
        None,
    )
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let mut stream = Stream::Synthetic(synthetic);
    finish(&mut stream, spec, asynchronous, &environment).await
}

async fn drive_cached(
    call: &Call,
    spec: &RunSpec,
    asynchronous: bool,
) -> Result<RunOutcome, SdkHttpError> {
    let environment = Environment::new()?;
    let hooks = environment.hooks();
    let api_base = NoApiBase;
    let headers = IndexMap::new();
    let response = response_of(spec)?;
    let model = response.model.clone().unwrap_or_default();
    let synthetic = SyntheticStream::build::<NoCost>(
        response,
        SyntheticSetup {
            hooks: &hooks,
            model: &model,
            custom_llm_provider: Some("cached_response"),
            litellm_metadata: None,
            request_data: None,
            call_type: None,
            headers: &headers,
            api_base: &api_base,
            logging: &environment.logging,
            times: times(),
            cache_hit: Some(true),
            persist_before_logging: false,
        },
        None,
    )
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let mut stream = Stream::Synthetic(synthetic);
    finish(&mut stream, spec, asynchronous, &environment).await
}

async fn finish(
    stream: &mut Stream<'_>,
    spec: &RunSpec,
    asynchronous: bool,
    environment: &Environment,
) -> Result<RunOutcome, SdkHttpError> {
    let (steps, failure) = drive_stream(stream, spec, asynchronous).await?;
    stream.drain_logging().await;
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
    asynchronous: bool,
) -> Result<(Vec<StepOutcome>, Option<Failure>), SdkHttpError> {
    let mut steps = Vec::new();
    for step in &spec.steps {
        let (outcome, failure) = drive_step(stream, *step, asynchronous).await?;
        steps.push(outcome);
        if let Some(projected) = failure {
            return Ok((steps, Some(projected)));
        }
    }
    Ok((steps, None))
}

async fn drive_step(
    stream: &mut Stream<'_>,
    step: Step,
    asynchronous: bool,
) -> Result<(StepOutcome, Option<Failure>), SdkHttpError> {
    let mut events = Vec::new();
    loop {
        let outcome = if asynchronous {
            stream.next_async().await
        } else {
            stream.next_blocking()
        };
        match outcome {
            Ok(Some(event)) => {
                events.push(view(&event)?);
                if matches!(step, Step::Next) {
                    return Ok((StepOutcome { events, end: false }, None));
                }
            }
            Ok(None) => return Ok((StepOutcome { events, end: true }, None)),
            Err(error) => {
                return Ok((
                    StepOutcome { events, end: false },
                    Some(failure_of(&error)?),
                ));
            }
        }
    }
}

fn times() -> StreamTimes {
    let now = DateTime::UNIX_EPOCH.fixed_offset();
    StreamTimes {
        now,
        created_timestamp: 0.0,
    }
}

fn response_of(spec: &RunSpec) -> Result<ResponsesApiResponse, SdkHttpError> {
    let response = spec
        .response
        .as_ref()
        .ok_or_else(|| SdkHttpError::Invalid("a fake or cached run needs a response".to_owned()))?;
    let value = serde_json::to_value(response).map_err(SdkHttpError::Serialize)?;
    serde_json::from_value(value).map_err(SdkHttpError::ResultParse)
}
