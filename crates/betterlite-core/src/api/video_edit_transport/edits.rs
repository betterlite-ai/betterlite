use tokio::runtime::{Builder, Handle};

use super::calls::{Handled, VideoEditRequest};
use super::config::{BaseVideoConfig, Params, VideoEditInput, VideoReply};
use super::error::VideoEditTransportError;
use super::handler::{Prepared, VideoTransport};
use super::seams::{VideoClientSource, VideoErrorConvert, VideoLoggingHandle, VideoPreCall};
use super::wire::{SUCCESS_LIMIT, display_facts, json_request, multipart_request, status_facts};
use crate::api::vector_store_http_write::service::{ClientKind, WriteRequest, WriteTransport as _};
use crate::api::video_records::VideoObject;

struct EditReady {
    wire: WriteRequest,
    data: Params,
}

impl<Source, Log, Convert> VideoTransport<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    // LiteLLM's video_edit_handler: the async handler's future when _is_async is set, and otherwise this thread blocks on it
    pub fn video_edit<'call>(
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
        let outcome = runtime.block_on(async {
            let prepared = self.prepare(
                request.client.as_ref(),
                request.fields(),
                config,
                ClientKind::Sync,
            )?;
            let prefetched = self.prefetch(&prepared, &request, config).await?;
            let ready = self.begin_edit(&prepared, &request, config, prefetched.as_ref())?;
            self.deliver_edit(&prepared, &request, config, ready).await
        });
        outcome.map(|object| Handled::Done(Box::new(object)))
    }

    pub async fn async_video_edit<'call>(
        &'call self,
        request: VideoEditRequest<'call, Source::Client>,
        config: &'call dyn BaseVideoConfig,
    ) -> Result<VideoObject, VideoEditTransportError> {
        let prepared = self.prepare(
            request.client.as_ref(),
            request.fields(),
            config,
            ClientKind::Asynchronous,
        )?;
        let prefetched = self.prefetch(&prepared, &request, config).await?;
        let ready = self.begin_edit(&prepared, &request, config, prefetched.as_ref())?;
        self.deliver_edit(&prepared, &request, config, ready).await
    }

    // The edit's own first try block: the prefetch POST and its status, while the .json() decode sits after it
    async fn prefetch(
        &self,
        prepared: &Prepared<'_, Source::Client>,
        request: &VideoEditRequest<'_, Source::Client>,
        config: &dyn BaseVideoConfig,
    ) -> Result<Option<Params>, VideoEditTransportError> {
        let Some((url, body)) = config.get_video_edit_prefetch_params(
            request.video_id,
            prepared.context(request.litellm_params),
        ) else {
            return Ok(None);
        };
        let wire = json_request(&url, &prepared.headers, &body, request.timeout)
            .map_err(|failure| self.provider_error(config, &display_facts(&failure.to_string())))?;
        let reply =
            prepared.client.get().send(wire).await.map_err(|failure| {
                self.provider_error(config, &display_facts(&failure.to_string()))
            })?;
        if reply.status >= SUCCESS_LIMIT {
            return Err(self.provider_error(config, &status_facts(&reply)));
        }
        let data: Params =
            serde_json::from_slice(&reply.body).map_err(VideoEditTransportError::PrefetchDecode)?;
        Ok(Some(data))
    }

    // The edit's main try block, up to the request: the transform and the pre_call are converted here
    fn begin_edit<'call>(
        &self,
        prepared: &Prepared<'call, Source::Client>,
        request: &VideoEditRequest<'call, Source::Client>,
        config: &dyn BaseVideoConfig,
        prefetched: Option<&Params>,
    ) -> Result<EditReady, VideoEditTransportError> {
        let (url, data, files) = config
            .transform_video_edit_request(
                VideoEditInput {
                    prompt: request.prompt,
                    video_id: request.video_id,
                    video_file: request.video_file,
                    extra_body: request.extra_body,
                    prefetched_source_data: prefetched,
                },
                prepared.context(request.litellm_params),
            )
            .map_err(|source| self.provider_error(config, &display_facts(&source.to_string())))?;
        self.logging
            .pre_call(&VideoPreCall {
                input: request.prompt,
                api_key: "",
                complete_input_dict: &data,
                api_base: &url,
                headers: &prepared.headers,
                video_id: request.video_id,
            })
            .map_err(|source| self.provider_error(config, &display_facts(&source.to_string())))?;
        // a non-empty files list sends multipart, and an empty one is falsy in LiteLLM and sends JSON
        let built = match files.as_ref().filter(|list| !list.is_empty()) {
            Some(files) => {
                multipart_request(&url, &prepared.headers, &data, files, request.timeout)
            }
            None => json_request(&url, &prepared.headers, &data, request.timeout),
        };
        let wire = built
            .map_err(|failure| self.provider_error(config, &display_facts(&failure.to_string())))?;
        Ok(EditReady { wire, data })
    }

    // The rest of the edit's main try block: the send, raise_for_status and the response transform
    async fn deliver_edit(
        &self,
        prepared: &Prepared<'_, Source::Client>,
        request: &VideoEditRequest<'_, Source::Client>,
        config: &dyn BaseVideoConfig,
        ready: EditReady,
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
        let mut object = self.logging.logging_object();
        config
            .transform_video_edit_response(
                VideoReply {
                    raw_response: &reply,
                    logging_obj: &mut object,
                    custom_llm_provider: Some(request.provider),
                },
                Some(&ready.data),
            )
            .map_err(|_source| self.provider_error(config, &status_facts(&reply)))
    }
}
