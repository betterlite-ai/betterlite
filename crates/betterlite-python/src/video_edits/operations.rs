use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_core::api::video_character_calls::error::{
    CharacterCallError, CharacterCallFailure, failure,
};
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
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;

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
    let Input {
        session,
        wire,
        request,
        video,
    } = input;
    if request.mock_response.is_none() {
        refuse_other_providers(
            &request.custom_llm_provider,
            Which::Create,
            &session.redactor,
        )?;
    }
    let config = config(&session, &wire.settings, video.and_then(|file| file.name));
    let mut clients = session.clients();
    let mut state = logging(&session.redactor, CREATE_CALL_TYPE)?;
    let sink = SilentSink;
    let env = pre_post(&sink, &session.redactor);
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let deps = CharacterCallDeps {
        redactor: &session.redactor,
        request_timeout_seconds: session.request_timeout_seconds,
    };
    avideo_create_character(&mut transport, deps, &request)
        .await
        .map_err(RunError::from)
}

// video_get_character and avideo_get_character: the mock first, then the one lookup
pub(crate) async fn get_character(
    input: Input<GetCharacterCall>,
) -> Result<CharacterObject, RunError> {
    let Input {
        session,
        wire,
        request,
        ..
    } = input;
    if request.mock_response.is_none() {
        refuse_other_providers(&request.custom_llm_provider, Which::Get, &session.redactor)?;
    }
    let config = config(&session, &wire.settings, None);
    let mut clients = session.clients();
    let mut state = logging(&session.redactor, GET_CALL_TYPE)?;
    let sink = SilentSink;
    let env = pre_post(&sink, &session.redactor);
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);
    let deps = CharacterCallDeps {
        redactor: &session.redactor,
        request_timeout_seconds: session.request_timeout_seconds,
    };
    avideo_get_character(&mut transport, deps, &request)
        .await
        .map_err(RunError::from)
}

// video_edit and avideo_edit: the mock, the provider the id or the caller names, then one POST
pub(crate) async fn edit(input: Input<OwnedEdit>) -> Result<VideoObject, RunError> {
    let Input {
        session,
        wire,
        request,
        video,
    } = input;
    let config = config(&session, &wire.settings, video.and_then(|file| file.name));
    let source = EditSource::new(&session);
    let log = SilentLog;
    let converter = SharedErrorConvert;
    let transport = VideoTransport::new(&source, &log, &converter);
    let state = Mutex::new(logging(&session.redactor, EDIT_CALL_TYPE)?);
    let deps = VideoEditDeps {
        configs: &config,
        handler: &transport,
        logging: &state,
        redactor: &session.redactor,
        request_timeout: request_timeout(&session)?,
        client: None,
    };
    let call = request.call();
    avideo_edit(&deps, &call).await.map_err(RunError::from)
}

// video_extension and avideo_extension: the mock, the provider, then one JSON POST
pub(crate) async fn extension(input: Input<OwnedExtension>) -> Result<VideoObject, RunError> {
    let Input {
        session,
        wire,
        request,
        ..
    } = input;
    let config = config(&session, &wire.settings, None);
    let source = EditSource::new(&session);
    let log = SilentLog;
    let converter = SharedErrorConvert;
    let transport = VideoTransport::new(&source, &log, &converter);
    let state = Mutex::new(logging(&session.redactor, EXTENSION_CALL_TYPE)?);
    let deps = VideoExtensionDeps {
        configs: &config,
        handler: &transport,
        logging: &state,
        redactor: &session.redactor,
        request_timeout: request_timeout(&session)?,
        client: None,
    };
    let call = request.call();
    avideo_extension(&deps, &call).await.map_err(RunError::from)
}
