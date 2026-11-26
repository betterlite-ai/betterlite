use std::time::Duration;

use betterlite_core::api::video_content_call::VideoContentCaller;
use betterlite_core::api::video_content_call::handler::ContentTransportHandler;
use betterlite_core::api::video_generation_call::call::{
    GenerationCallEnvironment, VideoGenerationCaller,
};
use betterlite_core::api::video_generation_call::error::{
    VideoGenerationCallError, failure as generation_failure,
};
use betterlite_core::api::video_generation_call::handler::TransportGenerationHandler;
use betterlite_core::api::video_generation_call::{
    DEFAULT_MODEL, VideoGenerationRequest, mock_video_response,
};
use betterlite_core::api::video_list_call::{ListRequest, VideoListReply, avideo_list, video_list};
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_call::shared::VideoCallDeps;
use betterlite_core::api::video_remix_call::{
    RemixRequest, VideoRemixCall, avideo_remix, video_remix,
};
use betterlite_core::api::video_remix_list_transport::VideoRemixListTransport;
use betterlite_core::api::video_status_call::VideoStatusCaller;
use betterlite_core::api::video_status_call::handler::VideoStatusHandler;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::adapter::OpenAiVideoConfig;
use super::clients::{NativeSource, StatusConverter, StatusFactories};
use super::convert::{ConvertError, Reference, Requests, VideoCallWire, drop_params};
use super::error::{RunError, SessionError};
use super::providers::OpenAiProviders;
use super::session::{SilentSink, VideoSession, config, logging, pre_post};

const GENERATION_CALL_TYPE: &str = "video_generation";
const CONTENT_CALL_TYPE: &str = "video_content";
const STATUS_CALL_TYPE: &str = "video_retrieve";
const REMIX_CALL_TYPE: &str = "video_remix";
const LIST_CALL_TYPE: &str = "video_list";

// Which Python entry point asked: the sync one answers a mock before it infers a provider
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Sync,
    Async,
}

fn providers(
    session: &VideoSession,
    wire: &VideoCallWire,
    media: Option<Reference>,
) -> OpenAiProviders {
    OpenAiProviders {
        config: config(session, &wire.settings, media),
        source: source(session),
    }
}

fn source(session: &VideoSession) -> NativeSource {
    NativeSource::new(&session.environment, &session.client)
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
pub(crate) async fn generate(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
    media: Option<Reference>,
    mode: Mode,
) -> Result<VideoObject, RunError> {
    let caller = VideoGenerationCaller {
        providers: providers(session, wire, media),
        handler: TransportGenerationHandler,
        redactor: redactor()?,
        request_timeout: request_timeout(session)?,
        drop_params: drop_params(wire)?,
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
pub(crate) async fn content(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
) -> Result<Vec<u8>, RunError> {
    let caller = VideoContentCaller {
        providers: providers(session, wire, None),
        handler: ContentTransportHandler,
        redactor: redactor()?,
        request_timeout: request_timeout(session)?,
    };
    let mut state = logging(&caller.redactor, CONTENT_CALL_TYPE)?;
    let request = requests.content.ok_or(ConvertError::Kind)?;
    caller
        .avideo_content(request, None, &mut state)
        .await
        .map_err(RunError::from)
}

// video_status and avideo_status: the mock, the provider, the logging update and the retrieve handler
pub(crate) async fn status(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
) -> Result<VideoObject, RunError> {
    let factories = StatusFactories {
        source: source(session),
    };
    let converter = StatusConverter;
    let redactor = redactor()?;
    let env_redactor = redactor()?;
    let mut state = logging(&redactor, STATUS_CALL_TYPE)?;
    let sink = SilentSink;
    let env = pre_post(&sink, &env_redactor);
    let caller = VideoStatusCaller {
        providers: providers(session, wire, None),
        handler: VideoStatusHandler {
            factories: &factories,
            logging: &env,
            converter: &converter,
        },
        redactor,
        request_timeout: request_timeout(session)?,
    };
    let request = requests.status.ok_or(ConvertError::Kind)?;
    caller
        .avideo_status(request, None, &mut state)
        .await
        .map_err(RunError::from)
}

// The redactor and the pre/post environment every remix and list call writes through
struct Harness {
    redactor: Redactor,
    sink: SilentSink,
}

impl Harness {
    fn new() -> Result<Self, RunError> {
        Ok(Self {
            redactor: redactor()?,
            sink: SilentSink,
        })
    }

    // the environment borrows this harness, so the wiring below can be borrowed mutably beside it
    fn environment(&self) -> PrePostEnv<'_> {
        pre_post(&self.sink, &self.redactor)
    }
}

// The collaborators one remix or list call borrows, all owned here so the entry can hand them out
struct Wiring<Call> {
    providers: OpenAiProviders,
    transport: VideoRemixListTransport<NativeSource>,
    state: LoggingState,
    call: Call,
}

// What the transport's request needs, taken from one mutable borrow of the wiring
struct Parts<'deps, 'env, Call> {
    deps: VideoCallDeps<'deps, 'env>,
    transport: &'deps VideoRemixListTransport<NativeSource>,
    call: &'deps Call,
}

impl<Call> Wiring<Call> {
    fn new(
        session: &VideoSession,
        wire: &VideoCallWire,
        harness: &Harness,
        call_type: &str,
        call: Call,
    ) -> Result<Self, RunError> {
        Ok(Self {
            providers: providers(session, wire, None),
            transport: VideoRemixListTransport {
                source: source(session),
            },
            state: logging(&harness.redactor, call_type)?,
            call,
        })
    }

    fn parts<'deps, 'env>(
        &'deps mut self,
        harness: &'deps Harness,
        env: &'deps PrePostEnv<'env>,
        session: &VideoSession,
    ) -> Parts<'deps, 'env, Call> {
        Parts {
            deps: VideoCallDeps {
                configs: &self.providers,
                logging: &mut self.state,
                env,
                redactor: &harness.redactor,
                request_timeout_seconds: session.request_timeout_seconds,
            },
            transport: &self.transport,
            call: &self.call,
        }
    }
}

// video_remix: the transport's blocking client answers on this thread, so the call runs here and is never awaited
pub(crate) fn remix(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
) -> Result<VideoObject, RunError> {
    let harness = Harness::new()?;
    let env = harness.environment();
    let mut wiring = Wiring::new(
        session,
        wire,
        &harness,
        REMIX_CALL_TYPE,
        requests.remix.ok_or(ConvertError::Kind)?,
    )?;
    let parts = wiring.parts(&harness, &env, session);
    let request = RemixRequest {
        transport: parts.transport,
        deps: parts.deps,
        call: parts.call,
        client: None,
    };
    video_remix(request).map_err(RunError::from)
}

// avideo_remix: the mock, then the async handler's future awaited here
pub(crate) async fn aremix(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
) -> Result<VideoObject, RunError> {
    let harness = Harness::new()?;
    let env = harness.environment();
    let mut wiring = Wiring::new(
        session,
        wire,
        &harness,
        REMIX_CALL_TYPE,
        requests.remix.ok_or(ConvertError::Kind)?,
    )?;
    let parts = wiring.parts(&harness, &env, session);
    let request = RemixRequest {
        transport: parts.transport,
        deps: parts.deps,
        call: parts.call,
        client: None,
    };
    avideo_remix(request).await.map_err(RunError::from)
}

// video_list: openai unless the caller named a provider, and the page or the mock's objects as the answer
pub(crate) fn list(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
) -> Result<VideoListReply, RunError> {
    let harness = Harness::new()?;
    let env = harness.environment();
    let mut wiring = Wiring::new(
        session,
        wire,
        &harness,
        LIST_CALL_TYPE,
        requests.list.ok_or(ConvertError::Kind)?,
    )?;
    let parts = wiring.parts(&harness, &env, session);
    let request = ListRequest {
        transport: parts.transport,
        deps: parts.deps,
        call: parts.call,
        client: None,
    };
    video_list(request).map_err(RunError::from)
}

// avideo_list: its provider inference reads the empty model, as LiteLLM's async list does
pub(crate) async fn alist(
    session: &VideoSession,
    requests: Requests,
    wire: &VideoCallWire,
) -> Result<VideoListReply, RunError> {
    let harness = Harness::new()?;
    let env = harness.environment();
    let mut wiring = Wiring::new(
        session,
        wire,
        &harness,
        LIST_CALL_TYPE,
        requests.list.ok_or(ConvertError::Kind)?,
    )?;
    let parts = wiring.parts(&harness, &env, session);
    let request = ListRequest {
        transport: parts.transport,
        deps: parts.deps,
        call: parts.call,
        client: None,
    };
    let resolution = session.resolution();
    avideo_list(request, &resolution)
        .await
        .map_err(RunError::from)
}
