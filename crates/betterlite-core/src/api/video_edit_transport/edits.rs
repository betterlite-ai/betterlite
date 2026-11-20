use std::future::Future;

use betterlite_http::proxy::UpstreamResponse;
use tokio::runtime::{Builder, Handle};

use super::calls::{Handled, VideoEditRequest};
use super::config::{BaseVideoConfig, Params, VideoEditInput, VideoReply};
use super::error::{StepFailure, VideoEditTransportError};
use super::handler::{PrepareFields, Prepared, VideoTransport};
use super::seams::{VideoClientSource, VideoErrorConvert, VideoLoggingHandle, VideoPreCall};
use super::wire::{SUCCESS_LIMIT, display_facts, status_facts, video_request};
use crate::api::vector_store_http_write::service::{ClientKind, WriteRequest, WriteTransport as _};
use crate::api::video_records::VideoObject;

// The workspace denies multiple_inherent_impl, so the edit handler pair is a trait on the transport
pub trait VideoEditHandlers {
    type Client;

    fn video_edit<'call>(
        &'call self,
        request: VideoEditRequest<'call, Self::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<Handled<'call>, VideoEditTransportError>;

    fn async_video_edit<'call>(
        &'call self,
        request: VideoEditRequest<'call, Self::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> impl Future<Output = Result<VideoObject, VideoEditTransportError>> + Send + 'call;
}

struct EditReady {
    wire: WriteRequest,
    data: Params,
}

// One edit call bound to its seams and config, so the steps take only the call's own values
struct Edit<'call, Source: ?Sized, Log: ?Sized, Convert: ?Sized>
where
    Source: VideoClientSource,
    Log: VideoLoggingHandle,
    Convert: VideoErrorConvert,
{
    transport: &'call VideoTransport<'call, Source, Log, Convert>,
    config: &'call dyn BaseVideoConfig,
}

impl<Source, Log, Convert> VideoEditHandlers for VideoTransport<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    type Client = Source::Client;

    // LiteLLM's video_edit_handler: the async handler's future when _is_async is set, and otherwise this thread blocks on it
    fn video_edit<'call>(
        &'call self,
        request: VideoEditRequest<'call, Source::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<Handled<'call>, VideoEditTransportError> {
        if request.is_async {
            return Ok(Handled::Pending(Box::pin(
                self.async_video_edit(request, config),
            )));
        }
        if Handle::try_current().is_ok() {
            return Err(VideoEditTransportError::SyncInRuntime { status_code: 500 });
        }
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(VideoEditTransportError::Runtime)?;
        let edit = Edit {
            transport: self,
            config,
        };
        let object = runtime.block_on(edit.run(request, ClientKind::Sync))?;
        Ok(Handled::Done(Box::new(object)))
    }

    async fn async_video_edit<'call>(
        &'call self,
        request: VideoEditRequest<'call, Source::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<VideoObject, VideoEditTransportError> {
        Edit {
            transport: self,
            config,
        }
        .run(request, ClientKind::Asynchronous)
        .await
    }
}

impl<Source, Log, Convert> Edit<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    // The async handler's body, run by both paths with the client kind the path needs
    async fn run(
        &self,
        request: VideoEditRequest<'_, Source::Client>,
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
        let prefetched = self.prefetch(&prepared, &request).await?;
        let ready = self
            .begin(&prepared, &request, prefetched.as_ref())
            .map_err(|cause| {
                self.transport
                    .provider_error(self.config, &display_facts(&cause.to_string()))
            })?;
        self.deliver(prepared.client.get(), request.provider, ready)
            .await
    }

    // The edit's own first try block: the prefetch POST and its status, while the .json() decode sits after it
    async fn prefetch(
        &self,
        prepared: &Prepared<'_, Source::Client>,
        request: &VideoEditRequest<'_, Source::Client>,
    ) -> Result<Option<Params>, VideoEditTransportError> {
        let transport = self.transport;
        let config = self.config;
        let Some((url, body)) = config.get_video_edit_prefetch_params(
            request.video_id,
            prepared.context(request.litellm_params),
        ) else {
            return Ok(None);
        };
        let wire = video_request(&url, &prepared.headers, &body, None, request.timeout).map_err(
            |cause| transport.provider_error(config, &display_facts(&cause.to_string())),
        )?;
        let reply = self.post(prepared.client.get(), wire).await?;
        let data: Params =
            serde_json::from_slice(&reply.body).map_err(VideoEditTransportError::PrefetchDecode)?;
        Ok(Some(data))
    }

    // The edit's main try block up to the request: whichever step fails becomes the provider's error
    fn begin(
        &self,
        prepared: &Prepared<'_, Source::Client>,
        request: &VideoEditRequest<'_, Source::Client>,
        prefetched: Option<&Params>,
    ) -> Result<EditReady, StepFailure> {
        let (url, data, upload_files) = self.config.transform_video_edit_request(
            VideoEditInput {
                prompt: request.prompt,
                video_id: request.video_id,
                video_file: request.video_file,
                extra_body: request.extra_body,
                prefetched_source_data: prefetched,
            },
            prepared.context(request.litellm_params),
        )?;
        self.transport.logging.pre_call(&VideoPreCall {
            input: request.prompt,
            api_key: "",
            complete_input_dict: &data,
            api_base: &url,
            headers: &prepared.headers,
            video_id: request.video_id,
        })?;
        // a non-empty files list sends multipart, and an empty one is falsy in LiteLLM and sends JSON
        let files = upload_files.as_ref().filter(|list| !list.is_empty());
        let wire = video_request(&url, &prepared.headers, &data, files, request.timeout)?;
        Ok(EditReady { wire, data })
    }

    // The rest of the edit's main try block: the send, raise_for_status and the response transform
    async fn deliver(
        &self,
        client: &Source::Client,
        provider: &str,
        ready: EditReady,
    ) -> Result<VideoObject, VideoEditTransportError> {
        let transport = self.transport;
        let config = self.config;
        let reply = self.post(client, ready.wire).await?;
        let mut object = transport.logging.logging_object();
        config
            .transform_video_edit_response(
                VideoReply {
                    raw_response: &reply,
                    logging_obj: &mut object,
                    custom_llm_provider: Some(provider),
                },
                Some(&ready.data),
            )
            .map_err(|_cause| transport.provider_error(config, &status_facts(&reply)))
    }

    // One POST of the edit: the timeout rides on the wire, and any 400 or more raises the provider's error
    async fn post(
        &self,
        client: &Source::Client,
        wire: WriteRequest,
    ) -> Result<UpstreamResponse, VideoEditTransportError> {
        let transport = self.transport;
        let config = self.config;
        let reply = client.send(wire).await.map_err(|cause| {
            transport.provider_error(config, &display_facts(&cause.to_string()))
        })?;
        if reply.status >= SUCCESS_LIMIT {
            return Err(transport.provider_error(config, &status_facts(&reply)));
        }
        Ok(reply)
    }
}
