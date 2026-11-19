use betterlite_core::api::responses_stream_driver::errors::DriverError;
use betterlite_core::api::responses_stream_driver::live::{LiveStream, StreamEngine};
use betterlite_core::api::responses_stream_driver::source::PrefetchedSource;
use betterlite_core::api::responses_stream_driver::synthetic::{SyntheticSetup, SyntheticStream};
use betterlite_core::api::responses_stream_events::{
    BaseStreamStateInput, LoggingSnapshot, StreamTimes,
};
use betterlite_core::api::responses_stream_hooks::failure::await_pending_logging;
use betterlite_core::api::responses_stream_hooks::ports::Dispatch;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use chrono::DateTime;
use indexmap::IndexMap;

use super::assemble::{Decoder, Environment, Event, NoApiBase, NoCost, NoTokenization};
use super::call::{Call, Mode, RunOutcome, RunSpec, Step, StepOutcome};
use super::source;
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
            Self::Synthetic(stream) => stream.advance(Dispatch::Spawned).await,
        }
    }
    fn next_blocking(&mut self) -> Result<Option<Event>, DriverError> {
        match self {
            Self::Live(stream) => futures_executor::block_on(stream.drive(Dispatch::Inline)),
            Self::Synthetic(stream) => futures_executor::block_on(stream.advance(Dispatch::Inline)),
        }
    }
    async fn drain_logging(&mut self) {
        match self {
            Self::Live(stream) => await_pending_logging(&mut stream.callback).await,
            Self::Synthetic(stream) => {
                let callback = &mut stream.callback;
                await_pending_logging(callback).await;
            }
        }
    }
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
    let no_api_base = NoApiBase;
    let transform = Decoder;
    let cached = matches!(spec.mode, Mode::Cached);
    let opened = match spec.mode {
        Mode::Live => Some(source::prefetch(client, call, api_base).await?),
        Mode::Scripted => Some(source::scripted(spec)),
        Mode::Fake | Mode::Cached => None,
    };
    let mut model = call.model.clone();
    let mut stream = match opened {
        Some(items) => Stream::Live(Box::new(LiveStream::new(
            StreamEngine {
                transform: Some(&transform),
                tokenization: None::<&NoTokenization>,
                calculator: None::<&NoCost>,
                hooks: &hooks,
                logging: &environment.logging,
                details: IndexMap::new(),
                limits: &environment.limits,
            },
            BaseStreamStateInput {
                model: &call.model,
                custom_llm_provider: Some("openai"),
                litellm_metadata: None,
                request_data: None,
                call_type: None,
                logging: LoggingSnapshot::read(&environment.logging),
                headers: &headers,
                api_base: &no_api_base,
                times: times(),
                max_streaming_duration_seconds: None,
            },
            Box::new(PrefetchedSource::new(items)),
        ))),
        None => {
            let response = spec.response.as_ref().ok_or_else(|| {
                SdkHttpError::Invalid("a fake or cached run needs a response".to_owned())
            })?;
            let value = serde_json::to_value(response).map_err(SdkHttpError::Serialize)?;
            let response: ResponsesApiResponse =
                serde_json::from_value(value).map_err(SdkHttpError::ResultParse)?;
            if cached {
                model = response.model.clone().unwrap_or_default();
            }
            let synthesized = SyntheticStream::build::<NoCost>(
                response,
                SyntheticSetup {
                    state: BaseStreamStateInput {
                        model: &model,
                        custom_llm_provider: Some(if cached {
                            "cached_response"
                        } else {
                            "openai"
                        }),
                        litellm_metadata: None,
                        request_data: None,
                        call_type: None,
                        logging: LoggingSnapshot::read(&environment.logging),
                        headers: &headers,
                        api_base: &no_api_base,
                        times: times(),
                        max_streaming_duration_seconds: None,
                    },
                    hooks: &hooks,
                    cache_hit: cached.then_some(true),
                    persist_before_logging: !cached,
                },
                None,
            )
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
            Stream::Synthetic(synthesized)
        }
    };
    let (steps, failure) = drive_stream(&mut stream, spec).await?;
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
) -> Result<(Vec<StepOutcome>, Option<Failure>), SdkHttpError> {
    let mut steps = Vec::new();
    for &step in &spec.steps {
        let mut events = Vec::new();
        let mut end = false;
        let mut failure = None;
        loop {
            let outcome = if spec.asynchronous {
                stream.next_async().await
            } else {
                stream.next_blocking()
            };
            match outcome {
                Ok(Some(event)) => {
                    events.push(view(&event)?);
                    if matches!(step, Step::Next) {
                        break;
                    }
                }
                Ok(None) => {
                    end = true;
                    break;
                }
                Err(error) => {
                    failure = Some(failure_of(&error)?);
                    break;
                }
            }
        }
        steps.push(StepOutcome { events, end });
        if failure.is_some() {
            return Ok((steps, failure));
        }
    }
    Ok((steps, None))
}
fn times() -> StreamTimes {
    StreamTimes {
        now: DateTime::UNIX_EPOCH.fixed_offset(),
        created_timestamp: 0.0,
    }
}
