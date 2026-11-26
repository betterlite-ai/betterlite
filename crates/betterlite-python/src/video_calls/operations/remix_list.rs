use betterlite_core::api::video_list_call::{VideoListReply, avideo_list, video_list};
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_call::shared::VideoCallDeps;
use betterlite_core::api::video_remix_call::{avideo_remix, video_remix};
use betterlite_core::api::video_remix_list_transport::VideoRemixListTransport;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;

use super::super::clients::NativeSource;
use super::super::convert::{ConvertError, VideoCallWire};
use super::super::error::RunError;
use super::super::providers::OpenAiProviders;
use super::super::session::{VideoSession, logging, pre_post};
use super::{Input, providers};

mod list_request;
mod remix_request;

const REMIX_CALL_TYPE: &str = "video_remix";
const LIST_CALL_TYPE: &str = "video_list";

// The collaborators one remix or list call borrows, all owned here so the entry can hand them out
struct Wiring<'session, Call> {
    session: &'session VideoSession,
    providers: OpenAiProviders,
    transport: VideoRemixListTransport<NativeSource>,
    state: LoggingState,
    env: PrePostEnv<'session>,
    call: Call,
}

// What the transport's request needs, taken from one mutable borrow of the wiring
pub(super) struct Parts<'deps, 'env, Call> {
    pub(super) deps: VideoCallDeps<'deps, 'env>,
    pub(super) transport: &'deps VideoRemixListTransport<NativeSource>,
    pub(super) call: &'deps Call,
}

impl<'session, Call> Wiring<'session, Call> {
    fn new(
        session: &'session VideoSession,
        wire: &VideoCallWire,
        call_type: &str,
        call: Call,
    ) -> Result<Self, RunError> {
        Ok(Self {
            session,
            providers: providers(session, wire, None),
            transport: VideoRemixListTransport {
                source: NativeSource::new(&session.environment, &session.client),
            },
            state: logging(&session.redactor, call_type)?,
            env: pre_post(&session.sink, &session.redactor),
            call,
        })
    }

    fn parts(&mut self) -> Parts<'_, 'session, Call> {
        Parts {
            deps: VideoCallDeps {
                configs: &self.providers,
                logging: &mut self.state,
                env: &self.env,
                redactor: &self.session.redactor,
                request_timeout_seconds: self.session.request_timeout_seconds,
            },
            transport: &self.transport,
            call: &self.call,
        }
    }
}

// video_remix: the transport's blocking client answers on this thread, so the call runs here and is never awaited
pub(crate) fn remix(input: Input) -> Result<VideoObject, RunError> {
    let call = input.requests.remix.ok_or(ConvertError::Kind)?;
    let mut wiring = Wiring::new(&input.session, &input.wire, REMIX_CALL_TYPE, call)?;
    video_remix(remix_request::build(wiring.parts())).map_err(RunError::from)
}

// avideo_remix: the mock, then the async handler's future awaited here
pub(crate) async fn aremix(input: Input) -> Result<VideoObject, RunError> {
    let call = input.requests.remix.ok_or(ConvertError::Kind)?;
    let mut wiring = Wiring::new(&input.session, &input.wire, REMIX_CALL_TYPE, call)?;
    avideo_remix(remix_request::build(wiring.parts()))
        .await
        .map_err(RunError::from)
}

// video_list: openai unless the caller named a provider, and the page or the mock's objects as the answer
pub(crate) fn list(input: Input) -> Result<VideoListReply, RunError> {
    let call = input.requests.list.ok_or(ConvertError::Kind)?;
    let mut wiring = Wiring::new(&input.session, &input.wire, LIST_CALL_TYPE, call)?;
    video_list(list_request::build(wiring.parts())).map_err(RunError::from)
}

// avideo_list: its provider inference reads the empty model, as LiteLLM's async list does
pub(crate) async fn alist(input: Input) -> Result<VideoListReply, RunError> {
    let call = input.requests.list.ok_or(ConvertError::Kind)?;
    let mut wiring = Wiring::new(&input.session, &input.wire, LIST_CALL_TYPE, call)?;
    let request = list_request::build(wiring.parts());
    avideo_list(request, &input.session.resolution())
        .await
        .map_err(RunError::from)
}
