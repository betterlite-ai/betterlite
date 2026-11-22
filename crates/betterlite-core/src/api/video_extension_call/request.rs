use std::time::Duration;

use super::VideoExtensionDeps;
use super::error::{VideoExtensionCallError, VideoExtensionFailure, failure};
use crate::api::vector_store_http_write::service::SuppliedClient;
use crate::api::video_edit_transport::calls::{Handled, VideoExtensionRequest, VideoPending};
use crate::api::video_edit_transport::config::{BaseVideoConfig, Headers, Params};
use crate::api::video_edit_transport::extensions::VideoExtensionHandlers;
use crate::api::video_records::VideoObject;

// Everything the handler call needs, owned so the pending future can outlive the caller's borrows
pub(crate) struct Ready<'run, Client> {
    pub(crate) provider: String,
    pub(crate) config: &'run dyn BaseVideoConfig,
    pub(crate) prompt: String,
    pub(crate) video_id: String,
    pub(crate) seconds: String,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) litellm_params: Params,
    pub(crate) timeout: Duration,
    pub(crate) is_async: bool,
    pub(crate) client: Option<SuppliedClient<'run, Client>>,
}

impl<Client> Ready<'_, Client> {
    // the handler call's arguments, borrowed from the owned fields
    pub(crate) fn request(&self) -> VideoExtensionRequest<'_, Client> {
        VideoExtensionRequest {
            prompt: &self.prompt,
            video_id: &self.video_id,
            seconds: &self.seconds,
            provider: &self.provider,
            litellm_params: &self.litellm_params,
            extra_headers: self.extra_headers.as_ref(),
            extra_body: self.extra_body.as_ref(),
            timeout: Some(self.timeout),
            client: self.client.as_ref().map(|client| SuppliedClient {
                kind: client.kind,
                transport: client.transport,
            }),
            api_key: None,
            is_async: self.is_async,
        }
    }
}

// the sync hand-off: the handler's answer turned into the call's result, with model "" on every failure
pub(crate) fn hand_off<'call, Configs, Handler>(
    deps: &'call VideoExtensionDeps<'_, Configs, Handler>,
    ready: &'call Ready<'_, Handler::Client>,
) -> Result<Box<VideoObject>, VideoExtensionFailure>
where
    Configs: ?Sized,
    Handler: VideoExtensionHandlers + ?Sized,
{
    match deps.handler.video_extension(ready.request(), ready.config) {
        Ok(Handled::Done(object)) => Ok(object),
        // the transport never answers Pending on its sync path, and a future borrowed from this call cannot leave it
        Ok(Handled::Pending(_pending)) => Err(failure(
            VideoExtensionCallError::SyncPending,
            &ready.provider,
            deps.redactor,
        )),
        Err(error) => Err(failure(
            VideoExtensionCallError::Handler(error),
            &ready.provider,
            deps.redactor,
        )),
    }
}

// the pending half of an async call, built from owned data so it outlives the caller's borrows
pub(crate) fn into_pending<'run, Handler>(
    ready: Ready<'run, Handler::Client>,
    handler: &'run Handler,
) -> VideoPending<'run>
where
    Handler: VideoExtensionHandlers + Sync + ?Sized,
    Handler::Client: Sync,
{
    Box::pin(async move {
        let config = ready.config;
        let request = ready.request();
        // errors travel as transport errors; avideo_extension maps them with the provider its caller named
        match handler.video_extension(request, config) {
            Ok(Handled::Done(object)) => Ok(*object),
            Ok(Handled::Pending(pending)) => pending.await,
            Err(error) => Err(error),
        }
    })
}
