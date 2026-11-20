use std::future::Future;

use tokio::runtime::{Builder, Handle};

use super::calls::{Handled, VideoExtensionRequest};
use super::config::{BaseVideoConfig, VideoExtensionInput, VideoLogging, VideoReply};
use super::error::VideoEditTransportError;
use super::handler::{PrepareFields, Prepared, VideoTransport};
use super::seams::{VideoClientSource, VideoErrorConvert, VideoLoggingHandle, VideoPreCall};
use super::wire::{SUCCESS_LIMIT, display_facts, status_facts, video_request};
use crate::api::vector_store_http_write::service::{ClientKind, WriteRequest, WriteTransport as _};
use crate::api::video_records::VideoObject;

// The workspace denies multiple_inherent_impl, so the extension handler pair is a trait on the transport
pub trait VideoExtensionHandlers {
    type Client;

    fn video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, Self::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<Handled<'call>, VideoEditTransportError>;

    fn async_video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, Self::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> impl Future<Output = Result<VideoObject, VideoEditTransportError>> + Send + 'call;
}

struct ExtensionReady {
    wire: WriteRequest,
    object: VideoLogging,
}

// One extension call bound to its seams and config, so the steps take only the call's own values
struct Extension<'call, Source: ?Sized, Log: ?Sized, Convert: ?Sized>
where
    Source: VideoClientSource,
    Log: VideoLoggingHandle,
    Convert: VideoErrorConvert,
{
    transport: &'call VideoTransport<'call, Source, Log, Convert>,
    config: &'call dyn BaseVideoConfig,
}

impl<Source, Log, Convert> VideoExtensionHandlers for VideoTransport<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    type Client = Source::Client;

    // LiteLLM's video_extension_handler: the async handler's future when _is_async is set, and otherwise this thread blocks on it
    fn video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, Source::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<Handled<'call>, VideoEditTransportError> {
        if request.is_async {
            return Ok(Handled::Pending(Box::pin(
                self.async_video_extension(request, config),
            )));
        }
        if Handle::try_current().is_ok() {
            return Err(VideoEditTransportError::SyncInRuntime { status_code: 500 });
        }
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(VideoEditTransportError::Runtime)?;
        let extension = Extension {
            transport: self,
            config,
        };
        let object = runtime.block_on(extension.run(request, ClientKind::Sync))?;
        Ok(Handled::Done(Box::new(object)))
    }

    async fn async_video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, Source::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<VideoObject, VideoEditTransportError> {
        Extension {
            transport: self,
            config,
        }
        .run(request, ClientKind::Asynchronous)
        .await
    }
}

impl<Source, Log, Convert> Extension<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    // The async handler's body, run by both paths with the client kind the path needs
    async fn run(
        &self,
        request: VideoExtensionRequest<'_, Source::Client>,
        mode: ClientKind,
    ) -> Result<VideoObject, VideoEditTransportError> {
        let prepared = self.transport.prepare(
            request.client.as_ref(),
            PrepareFields {
                provider: request.provider,
                litellm_params: request.litellm_params,
                extra_headers: request.extra_headers,
                api_key: request.api_key,
            },
            self.config,
            mode,
        )?;
        let ready = self.begin(&prepared, &request)?;
        self.deliver(prepared.client.get(), request.provider, ready)
            .await
    }

    // The transform and the pre_call sit before the try block here, so their errors are not converted
    fn begin(
        &self,
        prepared: &Prepared<'_, Source::Client>,
        request: &VideoExtensionRequest<'_, Source::Client>,
    ) -> Result<ExtensionReady, VideoEditTransportError> {
        let transport = self.transport;
        let config = self.config;
        let (url, data) = config
            .transform_video_extension_request(
                VideoExtensionInput {
                    prompt: request.prompt,
                    video_id: request.video_id,
                    seconds: request.seconds,
                    extra_body: request.extra_body,
                },
                prepared.context(request.litellm_params),
            )
            .map_err(VideoEditTransportError::Config)?;
        transport
            .logging
            .pre_call(&VideoPreCall {
                input: request.prompt,
                api_key: "",
                complete_input_dict: &data,
                api_base: &url,
                headers: &prepared.headers,
                video_id: request.video_id,
            })
            .map_err(VideoEditTransportError::Logging)?;
        let wire = video_request(&url, &prepared.headers, &data, None, request.timeout).map_err(
            |cause| transport.provider_error(config, &display_facts(&cause.to_string())),
        )?;
        Ok(ExtensionReady {
            wire,
            object: transport.logging.logging_object(),
        })
    }

    // The try block: the JSON POST with raise_for_status, then the response transform
    async fn deliver(
        &self,
        client: &Source::Client,
        provider: &str,
        ready: ExtensionReady,
    ) -> Result<VideoObject, VideoEditTransportError> {
        let transport = self.transport;
        let config = self.config;
        let reply = client.send(ready.wire).await.map_err(|cause| {
            transport.provider_error(config, &display_facts(&cause.to_string()))
        })?;
        if reply.status >= SUCCESS_LIMIT {
            return Err(transport.provider_error(config, &status_facts(&reply)));
        }
        let mut object = ready.object;
        config
            .transform_video_extension_response(VideoReply {
                raw_response: &reply,
                logging_obj: &mut object,
                custom_llm_provider: Some(provider),
            })
            .map_err(|_cause| transport.provider_error(config, &status_facts(&reply)))
    }
}
