use std::sync::Arc;
use std::time::Duration;

use betterlite_core::api::video_content_call::VideoContentCaller;
use betterlite_core::api::video_content_call::handler::ContentTransportHandler;
use betterlite_core::api::video_generation_call::call::{
    GenerationCallEnvironment, VideoGenerationCaller, mock_video_response,
};
use betterlite_core::api::video_generation_call::error::{
    VideoGenerationCallError, failure as generation_failure,
};
use betterlite_core::api::video_generation_call::handler::TransportGenerationHandler;
use betterlite_core::api::video_generation_call::{DEFAULT_MODEL, VideoGenerationRequest};
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_status_call::VideoStatusCaller;
use betterlite_core::api::video_status_transport::handler::VideoStatusHandler;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::clients::{NativeSource, StatusConverter, StatusFactories};
use super::convert::{ConvertError, Reference, Requests, VideoCallWire, drop_params};
use super::error::{RunError, SessionError};
use super::providers::OpenAiProviders;
use super::session::{SilentSink, VideoSession, config, logging, pre_post};

pub(crate) mod remix_list;

const GENERATION_CALL_TYPE: &str = "video_generation";
const CONTENT_CALL_TYPE: &str = "video_content";
const STATUS_CALL_TYPE: &str = "video_retrieve";

// Which Python entry point asked: the sync one answers a mock before it infers a provider
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Sync,
    Async,
}

// One call's native payload, taken apart as its operation reads it
pub(crate) type Parsed = (VideoCallWire, Requests, Option<Reference>);

// One call, as every operation takes it: the session it runs on and the payload Python sent
pub(crate) struct Input {
    pub(crate) session: Arc<VideoSession>,
    wire: VideoCallWire,
    requests: Requests,
    reference: Option<Reference>,
}

impl Input {
    pub(crate) fn new(session: Arc<VideoSession>, parsed: Parsed) -> Self {
        let (wire, requests, reference) = parsed;
        Self {
            session,
            wire,
            requests,
            reference,
        }
    }
}

fn providers(
    session: &VideoSession,
    wire: &VideoCallWire,
    media: Option<Reference>,
) -> OpenAiProviders {
    OpenAiProviders {
        config: config(session, &wire.settings, media),
        source: NativeSource::new(&session.environment, &session.client),
    }
}

fn redactor() -> Result<Redactor, RunError> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| RunError::from(SessionError::Redact { source }))
}

fn request_timeout(session: &VideoSession) -> Result<Duration, RunError> {
    Duration::try_from_secs_f64(session.request_timeout_seconds)
        .map_err(|source| RunError::from(SessionError::RequestTimeout(source)))
}

// video_generation and avideo_generation: the sync entry's own mock check, then the async call both entries end in
pub(crate) async fn generate(input: Input, mode: Mode) -> Result<VideoObject, RunError> {
    let Input {
        session,
        wire,
        requests,
        reference,
    } = input;
    let caller = VideoGenerationCaller {
        providers: providers(&session, &wire, reference),
        handler: TransportGenerationHandler,
        redactor: redactor()?,
        request_timeout: request_timeout(&session)?,
        drop_params: drop_params(&wire)?,
    };
    let mut state = logging(&caller.redactor, GENERATION_CALL_TYPE)?;
    let sink = SilentSink;
    let env = pre_post(&sink, &caller.redactor);
    let resolution = session.resolution();
    let request = requests.generation.ok_or(ConvertError::Kind)?;
    let environment = GenerationCallEnvironment {
        logging: &mut state,
        env: &env,
    };
    if matches!(mode, Mode::Sync) {
        if let Some(video) = mock_object(&caller, &request)? {
            return Ok(video);
        }
    }
    caller
        .avideo_generation(request, None, &resolution, environment)
        .await
        .map_err(RunError::from)
}

// LiteLLM's sync mock check: a mock answers before get_llm_provider, with the caller's own model and provider
fn mock_object(
    caller: &VideoGenerationCaller<OpenAiProviders, TransportGenerationHandler>,
    request: &VideoGenerationRequest,
) -> Result<Option<VideoObject>, RunError> {
    let model = request
        .model
        .as_deref()
        .filter(|model| !model.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    mock_video_response(&request.kwargs)
        .map_err(|error| {
            generation_failure(
                VideoGenerationCallError::Mock(error),
                Some(model),
                request.custom_llm_provider.as_deref(),
                &caller.redactor,
            )
        })
        .map_err(RunError::from)
}

// video_content and avideo_content: the provider comes from the video id, and the bytes are the answer
pub(crate) async fn content(input: Input) -> Result<Vec<u8>, RunError> {
    let Input {
        session,
        wire,
        requests,
        ..
    } = input;
    let caller = VideoContentCaller {
        providers: providers(&session, &wire, None),
        handler: ContentTransportHandler,
        redactor: redactor()?,
        request_timeout: request_timeout(&session)?,
    };
    let mut state = logging(&caller.redactor, CONTENT_CALL_TYPE)?;
    let request = requests.content.ok_or(ConvertError::Kind)?;
    caller
        .avideo_content(request, None, &mut state)
        .await
        .map_err(RunError::from)
}

// video_status and avideo_status: the mock, the provider, the logging update and the retrieve handler
pub(crate) async fn status(input: Input) -> Result<VideoObject, RunError> {
    let Input {
        session,
        wire,
        requests,
        ..
    } = input;
    let factories = StatusFactories {
        source: NativeSource::new(&session.environment, &session.client),
    };
    let converter = StatusConverter;
    let call_redactor = redactor()?;
    let env_redactor = redactor()?;
    let mut state = logging(&call_redactor, STATUS_CALL_TYPE)?;
    let sink = SilentSink;
    let env = pre_post(&sink, &env_redactor);
    let caller = VideoStatusCaller {
        providers: providers(&session, &wire, None),
        handler: VideoStatusHandler {
            factories: &factories,
            logging: &env,
            converter: &converter,
        },
        redactor: call_redactor,
        request_timeout: request_timeout(&session)?,
    };
    let request = requests.status.ok_or(ConvertError::Kind)?;
    caller
        .avideo_status(request, None, &mut state)
        .await
        .map_err(RunError::from)
}

mod tests;
