use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_core::api::video_character_calls::error::{
    CharacterCallError, CharacterCallFailure, failure,
};
use betterlite_core::api::video_character_calls::handler::CharacterHandler;
use betterlite_core::api::video_character_calls::{
    CharacterCallDeps, CreateCharacterCall, GetCharacterCall, avideo_create_character,
    avideo_get_character,
};
use betterlite_core::api::video_character_transport::service::CharacterTransport;
use betterlite_core::api::video_edit_call::{VideoEditDeps, avideo_edit};
use betterlite_core::api::video_edit_transport::handler::VideoTransport;
use betterlite_core::api::video_edit_transport::seams::SharedErrorConvert;
use betterlite_core::api::video_extension_call::{VideoExtensionDeps, avideo_extension};
use betterlite_core::api::video_records::{CharacterObject, VideoObject};
use betterlite_core::api::video_selection::VideoConfigTag;
use betterlite_core::api::video_selection::get_provider_video_config;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;

use super::adapter::OpenAiEditConfig;
use super::convert::requests::{OwnedEdit, OwnedExtension};
use super::convert::{VideoEditWire, VideoFile};
use super::error::{RunError, SessionError};
use super::session::{EditSession, EditSource, SilentLog, SilentSink, config, logging, pre_post};

const CREATE_CALL_TYPE: &str = "video_create_character";
const GET_CALL_TYPE: &str = "video_get_character";
const EDIT_CALL_TYPE: &str = "video_edit";
const EXTENSION_CALL_TYPE: &str = "video_extension";

// the wire and the operation's own request, plus the file the payload carried for it
pub(crate) type Parsed<Request> = (VideoEditWire, Request, Option<VideoFile>);

// one call as every operation takes it: the session it runs on and the payload Python sent
pub(crate) struct Input<Request> {
    pub(crate) session: Arc<EditSession>,
    wire: VideoEditWire,
    request: Request,
    video: Option<VideoFile>,
}

impl<Request> Input<Request> {
    pub(crate) fn new(session: Arc<EditSession>, parsed: Parsed<Request>) -> Self {
        let (wire, request, video) = parsed;
        Self {
            session,
            wire,
            request,
            video,
        }
    }
}

// which operation's wording an unsupported provider uses
#[derive(Clone, Copy)]
enum Which {
    Create,
    Get,
}

impl Which {
    const fn unsupported(self, provider: String) -> CharacterCallError {
        match self {
            Self::Create => CharacterCallError::CreateUnsupported { provider },
            Self::Get => CharacterCallError::GetUnsupported { provider },
        }
    }

    const fn call_type(self) -> &'static str {
        match self {
            Self::Create => CREATE_CALL_TYPE,
            Self::Get => GET_CALL_TYPE,
        }
    }
}

// a character request the session can run without knowing which of the two operations it came from
enum CharacterRequest {
    Create(CreateCharacterCall),
    Get(GetCharacterCall),
}

impl CharacterRequest {
    fn mock_response(&self) -> Option<&Json> {
        match self {
            Self::Create(call) => call.mock_response.as_ref(),
            Self::Get(call) => call.mock_response.as_ref(),
        }
    }

    fn provider(&self) -> &str {
        match self {
            Self::Create(call) => &call.custom_llm_provider,
            Self::Get(call) => &call.custom_llm_provider,
        }
    }

    const fn which(&self) -> Which {
        match self {
            Self::Create(_) => Which::Create,
            Self::Get(_) => Which::Get,
        }
    }

    async fn call<'run>(
        self,
        transport: &'run mut dyn CharacterHandler,
        deps: CharacterCallDeps<'run>,
    ) -> Result<CharacterObject, RunError> {
        match self {
            Self::Create(call) => avideo_create_character(transport, deps, &call)
                .await
                .map_err(RunError::from),
            Self::Get(call) => avideo_get_character(transport, deps, &call)
                .await
                .map_err(RunError::from),
        }
    }

    // one character call's setup and hand-off, for both operations
    async fn run(
        self,
        session: Arc<EditSession>,
        wire: VideoEditWire,
        video: Option<VideoFile>,
    ) -> Result<CharacterObject, RunError> {
        if self.mock_response().is_none() {
            refuse_other_providers(self.provider(), self.which(), &session.redactor)?;
        }
        let config = config(&session, &wire.settings, video.and_then(|file| file.name));
        let mut clients = session.clients();
        let mut state = logging(&session.redactor, self.which().call_type())?;
        let sink = SilentSink;
        let env = pre_post(&sink, &session.redactor);
        let mut transport =
            CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
        let deps = CharacterCallDeps {
            redactor: &session.redactor,
            request_timeout_seconds: session.request_timeout_seconds,
        };
        self.call(&mut transport, deps).await
    }
}

// the package builds OpenAI's config only, so every other provider is refused where LiteLLM has none
fn refuse_other_providers(
    provider: &str,
    operation: Which,
    redactor: &Redactor,
) -> Result<(), CharacterCallFailure> {
    let Ok(id) = provider.parse::<ProviderId>() else {
        return Ok(());
    };
    let Ok(Some(tag)) = get_provider_video_config(None, id) else {
        return Ok(());
    };
    if matches!(tag, VideoConfigTag::OpenAi) {
        return Ok(());
    }
    Err(failure(
        operation.unsupported(provider.to_owned()),
        provider,
        redactor,
    ))
}

fn request_timeout(session: &EditSession) -> Result<Duration, RunError> {
    Duration::try_from_secs_f64(session.request_timeout_seconds)
        .map_err(|source| RunError::from(SessionError::RequestTimeout(source)))
}

// video_create_character and avideo_create_character: the mock first, then the one multipart call
pub(crate) async fn create_character(
    input: Input<CreateCharacterCall>,
) -> Result<CharacterObject, RunError> {
    CharacterRequest::Create(input.request)
        .run(input.session, input.wire, input.video)
        .await
}

// video_get_character and avideo_get_character: the mock first, then the one lookup
pub(crate) async fn get_character(
    input: Input<GetCharacterCall>,
) -> Result<CharacterObject, RunError> {
    CharacterRequest::Get(input.request)
        .run(input.session, input.wire, input.video)
        .await
}

// the config, client source, log and logging state an edit or extension call runs on
struct Prepared {
    config: OpenAiEditConfig,
    source: EditSource,
    log: SilentLog,
    converter: SharedErrorConvert,
    state: Mutex<LoggingState>,
    timeout: Duration,
}

impl Prepared {
    fn new(
        session: &EditSession,
        wire: &VideoEditWire,
        video: Option<VideoFile>,
        call_type: &str,
    ) -> Result<Self, RunError> {
        Ok(Self {
            config: config(session, &wire.settings, video.and_then(|file| file.name)),
            source: EditSource::new(session),
            log: SilentLog,
            converter: SharedErrorConvert,
            state: Mutex::new(logging(&session.redactor, call_type)?),
            timeout: request_timeout(session)?,
        })
    }
}

// the owned request video_edit or video_extension runs
enum EditRequest {
    Edit(OwnedEdit),
    Extension(OwnedExtension),
}

impl EditRequest {
    const fn call_type(&self) -> &'static str {
        match self {
            Self::Edit(_) => EDIT_CALL_TYPE,
            Self::Extension(_) => EXTENSION_CALL_TYPE,
        }
    }

    async fn run(
        self,
        session: &EditSession,
        wire: &VideoEditWire,
        video: Option<VideoFile>,
    ) -> Result<VideoObject, RunError> {
        let prepared = Prepared::new(session, wire, video, self.call_type())?;
        let transport = VideoTransport::new(&prepared.source, &prepared.log, &prepared.converter);
        match self {
            Self::Edit(owned) => {
                let call = owned.call();
                let deps = VideoEditDeps {
                    configs: &prepared.config,
                    handler: &transport,
                    logging: &prepared.state,
                    redactor: &session.redactor,
                    request_timeout: prepared.timeout,
                    client: None,
                };
                avideo_edit(&deps, &call).await.map_err(RunError::from)
            }
            Self::Extension(owned) => {
                let call = owned.call();
                let deps = VideoExtensionDeps {
                    configs: &prepared.config,
                    handler: &transport,
                    logging: &prepared.state,
                    redactor: &session.redactor,
                    request_timeout: prepared.timeout,
                    client: None,
                };
                avideo_extension(&deps, &call).await.map_err(RunError::from)
            }
        }
    }
}

// video_edit and avideo_edit: the mock, the provider the id or the caller names, then one POST
pub(crate) async fn edit(input: Input<OwnedEdit>) -> Result<VideoObject, RunError> {
    EditRequest::Edit(input.request)
        .run(&input.session, &input.wire, input.video)
        .await
}

// video_extension and avideo_extension: the mock, the provider, then one JSON POST
pub(crate) async fn extension(input: Input<OwnedExtension>) -> Result<VideoObject, RunError> {
    EditRequest::Extension(input.request)
        .run(&input.session, &input.wire, input.video)
        .await
}
