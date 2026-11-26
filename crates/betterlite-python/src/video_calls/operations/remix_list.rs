use betterlite_core::api::video_list_call::{ListRequest, VideoListReply, avideo_list, video_list};
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_remix_call::shared::VideoCallDeps;
use betterlite_core::api::video_remix_call::{
    RemixRequest, VideoRemixCall, avideo_remix, video_remix,
};
use betterlite_core::api::video_remix_list_transport::VideoRemixListTransport;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;

use super::super::clients::NativeSource;
use super::super::convert::{ConvertError, Requests, VideoCallWire};
use super::super::error::RunError;
use super::super::providers::OpenAiProviders;
use super::super::session::{SilentSink, VideoSession, logging, pre_post};
use super::{providers, redactor, source};

const REMIX_CALL_TYPE: &str = "video_remix";
const LIST_CALL_TYPE: &str = "video_list";

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
