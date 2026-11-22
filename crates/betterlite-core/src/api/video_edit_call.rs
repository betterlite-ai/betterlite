pub mod error;
mod mock;
mod prepare;
mod request;
mod tests;

use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{VideoEditCallError, VideoEditFailure, failure};
use self::prepare::{Prepared, prepare};
use self::request::{hand_off, into_pending};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::vector_store_http_write::service::SuppliedClient;
use crate::api::video_edit_transport::calls::{Handled, VideoEditHandlers};
use crate::api::video_edit_transport::config::{BaseVideoConfig, Headers, Params};
use crate::api::video_records::VideoObject;
use crate::api::video_selection::VideoConfigTag;

pub(crate) const DEFAULT_PROVIDER: &str = "openai";
pub(crate) const CALL_TYPE: &str = "video_edit";
pub(crate) const LOGGING_OBJECT: &str = "litellm_logging_obj";
pub(crate) const ASYNC_CALL: &str = "async_call";
pub(crate) const LITELLM_CALL_ID: &str = "litellm_call_id";
pub(crate) const USER: &str = "user";
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

// The B3331 video configs a caller built, by the tag B3332's get_provider_video_config selects
pub trait VideoEditConfigLookup: Send + Sync {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn BaseVideoConfig>;
}

// The collaborators video_edit reads: the provider configs, the B3336 handler, B247's logging object and B185's redactor
pub struct VideoEditDeps<'deps, Configs: ?Sized, Handler: VideoEditHandlers + ?Sized> {
    pub configs: &'deps Configs,
    pub handler: &'deps Handler,
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // B198's request_timeout for `timeout or DEFAULT_REQUEST_TIMEOUT`, already resolved by the caller
    pub request_timeout: Duration,
    // the `client` loose keyword, which the handler uses only when its kind matches the path it runs
    pub client: Option<SuppliedClient<'deps, Handler::Client>>,
}

// LiteLLM's video_edit arguments, with the logging object left in the loose keywords where @client put it
#[derive(Clone, Copy)]
pub struct VideoEditCall<'call> {
    pub video_id: &'call str,
    pub prompt: &'call str,
    // the declared default is 600 seconds; None keeps it and Some(0) falls to B198's request timeout
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'call str>,
    pub extra_headers: Option<&'call Headers>,
    // accepted to match the signature, and LiteLLM never reads it
    pub extra_query: Option<&'call Params>,
    pub extra_body: Option<&'call Params>,
    // the keyword-only video file
    pub video: Option<&'call SuppliedMedia>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// LiteLLM's video_edit: the mock short circuit or the handler call, whose pending half the async wrapper awaits
pub fn video_edit<'run, Configs, Handler>(
    deps: &'run VideoEditDeps<'run, Configs, Handler>,
    call: &VideoEditCall<'_>,
) -> Result<Handled<'run>, VideoEditFailure>
where
    Configs: VideoEditConfigLookup + ?Sized,
    Handler: VideoEditHandlers + Sync + ?Sized,
    <Handler as VideoEditHandlers>::Client: Sync,
{
    match prepare(deps, call)? {
        Prepared::Mock(object) => Ok(Handled::Done(Box::new(object))),
        Prepared::Call(ready) if ready.is_async => {
            Ok(Handled::Pending(into_pending(ready, deps.handler)))
        }
        Prepared::Call(ready) => hand_off(deps, ready),
    }
}

// LiteLLM's avideo_edit: async_call true, the sync function with the same arguments, then the pending half awaited
pub async fn avideo_edit<'run, Configs, Handler>(
    deps: &'run VideoEditDeps<'run, Configs, Handler>,
    call: &VideoEditCall<'_>,
) -> Result<VideoObject, VideoEditFailure>
where
    Configs: VideoEditConfigLookup + ?Sized,
    Handler: VideoEditHandlers + Sync + ?Sized,
    <Handler as VideoEditHandlers>::Client: Sync,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.insert(ASYNC_CALL.to_owned(), Json::Bool(true));
    let flagged = VideoEditCall {
        kwargs: &kwargs,
        ..*call
    };
    let answer = video_edit(deps, &flagged).map_err(|error| error.remapped(deps.redactor))?;
    let pending = match answer {
        Handled::Done(object) => return Ok(*object),
        Handled::Pending(pending) => pending,
    };
    // an error raised inside the awaited future reaches this wrapper's except, which maps with the caller's provider
    let provider = call.custom_llm_provider.unwrap_or_default();
    pending
        .await
        .map_err(|error| failure(VideoEditCallError::Handler(error), provider, deps.redactor))
}
