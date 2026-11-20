use tokio::runtime::{Builder, Handle};

use super::calls::{Handled, VideoExtensionRequest};
use super::config::{BaseVideoConfig, VideoExtensionInput, VideoLogging, VideoReply};
use super::error::VideoEditTransportError;
use super::handler::{Prepared, VideoTransport};
use super::seams::{VideoClientSource, VideoErrorConvert, VideoLoggingHandle, VideoPreCall};
use super::wire::{SUCCESS_LIMIT, display_facts, json_request, status_facts};
use crate::api::vector_store_http_write::service::{ClientKind, WriteRequest, WriteTransport as _};
use crate::api::video_records::VideoObject;

struct ExtensionReady {
    wire: WriteRequest,
    object: VideoLogging,
}

impl<Source, Log, Convert> VideoTransport<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    // LiteLLM's video_extension_handler: the async handler's future when _is_async is set
    pub fn video_extension<'call>(
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
        let outcome = runtime.block_on(async {
            let prepared = self.prepare(
                request.client.as_ref(),
                request.fields(),
                config,
                ClientKind::Sync,
            )?;
            let ready = self.begin_extension(&prepared, &request, config)?;
            self.deliver_extension(&prepared, &request, config, ready)
                .await
        });
        outcome.map(|object| Handled::Done(Box::new(object)))
    }

    pub async fn async_video_extension<'call>(
        &'call self,
        request: VideoExtensionRequest<'call, Source::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<VideoObject, VideoEditTransportError> {
        let prepared = self.prepare(
            request.client.as_ref(),
            request.fields(),
            config,
            ClientKind::Asynchronous,
        )?;
        let ready = self.begin_extension(&prepared, &request, config)?;
        self.deliver_extension(&prepared, &request, config, ready)
            .await
    }

    // The transform and the pre_call sit before the try block here, so their errors are not converted
    fn begin_extension<'call>(
        &self,
        prepared: &Prepared<'call, Source::Client>,
        request: &VideoExtensionRequest<'call, Source::Client>,
        config: &dyn BaseVideoConfig,
    ) -> Result<ExtensionReady, VideoEditTransportError> {
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
        self.logging
            .pre_call(&VideoPreCall {
                input: request.prompt,
                api_key: "",
                complete_input_dict: &data,
                api_base: &url,
                headers: &prepared.headers,
                video_id: request.video_id,
            })
            .map_err(VideoEditTransportError::Logging)?;
        let wire = json_request(&url, &prepared.headers, &data, request.timeout)
            .map_err(|failure| self.provider_error(config, &display_facts(&failure.to_string())))?;
        Ok(ExtensionReady {
            wire,
            object: self.logging.logging_object(),
        })
    }

    // The try block: the JSON POST with raise_for_status, then the response transform
    async fn deliver_extension(
        &self,
        prepared: &Prepared<'_, Source::Client>,
        request: &VideoExtensionRequest<'_, Source::Client>,
        config: &dyn BaseVideoConfig,
        ready: ExtensionReady,
    ) -> Result<VideoObject, VideoEditTransportError> {
        let reply = prepared
            .client
            .get()
            .send(ready.wire)
            .await
            .map_err(|failure| self.provider_error(config, &display_facts(&failure.to_string())))?;
        if reply.status >= SUCCESS_LIMIT {
            return Err(self.provider_error(config, &status_facts(&reply)));
        }
        let mut object = ready.object;
        config
            .transform_video_extension_response(VideoReply {
                raw_response: &reply,
                logging_obj: &mut object,
                custom_llm_provider: Some(request.provider),
            })
            .map_err(|_source| self.provider_error(config, &status_facts(&reply)))
    }
}
