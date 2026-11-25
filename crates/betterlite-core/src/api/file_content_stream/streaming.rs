use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::stream::{
    FileContentStream as SdkFileStream, FileContentStreamingResult,
};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;

use crate::api::file_stream_logging::source::{
    AsyncByteSource, CloseCall, CloseResult, SourceFailure, StreamSource,
};
use crate::api::file_stream_logging::{FileContentStreamingResponse, FileStreamSetup};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

use super::error::FileContentError;
use super::plan::known_provider;
use super::service::{COMPATIBLE_PROVIDERS, ContentCredentials, ContentInputs, Reply, Selection};
use super::target::{
    CallerClient, ContentLogging as _, ContentOperation, ContentReply, ContentTarget,
    FileContentDispatch, StreamIdentity, SuppliedSdk,
};
use super::{ContentAnswer, ContentCall, ContentContext, FileContentStream, StreamedContent};

pub(super) trait FileContentStreaming<Dispatch: FileContentDispatch> {
    async fn streamed<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentAnswer, Dispatch>;

    fn target<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        client: Option<CallerClient<'call, Dispatch::Client>>,
        credentials: ContentCredentials<'call>,
    ) -> Reply<ContentTarget<'call, Dispatch::Config, Dispatch::Client>, Dispatch>;

    fn vertex_config(&self, selection: &Selection) -> Reply<Dispatch::Config, Dispatch>;

    async fn dispatch_streamed<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        target: ContentTarget<'call, Dispatch::Config, Dispatch::Client>,
    ) -> Reply<FileContentStreamingResult, Dispatch>;

    fn wrap(
        &self,
        call: &ContentCall<'_>,
        selection: &Selection,
        logging: Arc<Dispatch::Logging>,
        result: FileContentStreamingResult,
    ) -> ContentAnswer;
}

impl<Dispatch> FileContentStreaming<Dispatch> for FileContentStream<'_, Dispatch>
where
    Dispatch: FileContentDispatch,
{
    // file_content_streaming: write the identity, pick the provider branch, then let B1080 wrap the iterator
    async fn streamed<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentAnswer, Dispatch> {
        let ContentContext {
            trusted,
            client,
            logger,
            logging: supplied_logging,
        } = context;
        let logging = supplied_logging.unwrap_or_else(|| {
            self.dispatch
                .create_logging(&self.new_logger(call.kwargs, selection.mode))
        });
        let provider_name = selection
            .provider
            .clone()
            .unwrap_or_else(|| String::from("None"));
        logging.record_identity(&StreamIdentity {
            model: call
                .model
                .filter(|name| !name.is_empty())
                .unwrap_or_default(),
            custom_llm_provider: &provider_name,
            api_base: call.kwargs.api_base.as_deref(),
        });
        let target = self.target(
            call,
            selection,
            client,
            ContentCredentials { trusted, logger },
        )?;
        let result = self.dispatch_streamed(call, selection, target).await?;
        Ok(self.wrap(call, selection, logging, result))
    }

    // the streamed provider branch: the OpenAI-compatible names, then vertex, then the 400 every other name raises
    fn target<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        client: Option<CallerClient<'call, Dispatch::Client>>,
        credentials: ContentCredentials<'call>,
    ) -> Reply<ContentTarget<'call, Dispatch::Config, Dispatch::Client>, Dispatch> {
        let name = selection.provider.as_deref().unwrap_or_default();
        if COMPATIBLE_PROVIDERS.contains(&name) {
            return Ok(ContentTarget::Compatible(
                self.plan(call.kwargs, selection.mode)
                    .compatible(supplied_openai(client.as_ref())),
            ));
        }
        if name != "vertex_ai" {
            return Err(FileContentError::UnsupportedStreaming {
                provider: name.to_owned(),
            });
        }
        let config = self.vertex_config(selection)?;
        let ContentCredentials { trusted, logger } = credentials;
        Ok(ContentTarget::Generic(Box::new(self.generic(
            call,
            selection,
            config,
            ContentInputs {
                trusted,
                client: supplied_async_http(client),
                logger,
            },
        ))))
    }

    // vertex raises on the sync entry first, then needs a registered provider config
    fn vertex_config(&self, selection: &Selection) -> Reply<Dispatch::Config, Dispatch> {
        if !selection.mode.is_async {
            return Err(FileContentError::SyncVertexStreaming);
        }
        let id = known_provider::<Dispatch::Error>(selection.provider.as_deref())?;
        self.dispatch
            .files_config(id)
            .ok_or_else(|| FileContentError::UnsupportedStreaming {
                provider: selection.provider.clone().unwrap_or_default(),
            })
    }

    async fn dispatch_streamed<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        target: ContentTarget<'call, Dispatch::Config, Dispatch::Client>,
    ) -> Reply<FileContentStreamingResult, Dispatch> {
        let reply = self
            .dispatch
            .content(
                target,
                ContentOperation::Streamed {
                    request: &selection.request,
                    // httpx iter_bytes(chunk_size=0) raises, so a zero or unrepresentable size takes the default
                    chunk_size: usize::try_from(call.chunk_size)
                        .ok()
                        .and_then(NonZeroUsize::new),
                },
            )
            .await
            .map_err(FileContentError::Dispatch)?;
        match reply {
            ContentReply::Streamed(result) => Ok(*result),
            ContentReply::Buffered(_content) => Err(FileContentError::Reply),
        }
    }

    // B1080's wrapper: the provider's headers and exit ride beside the async SDK stream and its logger
    fn wrap(
        &self,
        call: &ContentCall<'_>,
        selection: &Selection,
        logging: Arc<Dispatch::Logging>,
        result: FileContentStreamingResult,
    ) -> ContentAnswer {
        let FileContentStreamingResult {
            stream, headers, ..
        } = result;
        let response = FileContentStreamingResponse::new(
            FileStreamSetup {
                source: StreamSource::Async(Box::new(SdkContentStream { stream })),
                file_id: call.file_id.to_owned(),
                model: call.model.map(str::to_owned),
                custom_llm_provider: selection.provider.clone(),
                logger: Some(logging.stream_logger()),
            },
            Arc::new(ContentClock(self.clock)),
        );
        ContentAnswer::Streamed(Box::new(StreamedContent { response, headers }))
    }
}

const fn supplied_openai<Client>(client: Option<&CallerClient<'_, Client>>) -> Option<SuppliedSdk> {
    match client {
        Some(CallerClient::Sdk(SuppliedSdk::OpenAi)) => Some(SuppliedSdk::OpenAi),
        Some(CallerClient::Sdk(SuppliedSdk::Azure) | CallerClient::Http(_)) | None => None,
    }
}

// vertex keeps the caller's client only as an AsyncHTTPHandler
fn supplied_async_http<Client>(
    client: Option<CallerClient<'_, Client>>,
) -> Option<SuppliedClient<'_, Client>> {
    match client {
        Some(CallerClient::Http(handler)) if handler.kind == ClientKind::Asynchronous => {
            Some(handler)
        }
        Some(CallerClient::Http(_) | CallerClient::Sdk(_)) | None => None,
    }
}

pub(super) struct ContentClock(pub(super) fn() -> DateTime<FixedOffset>);

impl SuccessClock for ContentClock {
    fn now(&self) -> DateTime<FixedOffset> {
        (self.0)()
    }
}

// The SDK stream as B1080's async source: next_chunk reads one re-chunked piece, and aclose has no method
struct SdkContentStream {
    stream: SdkFileStream,
}

#[async_trait]
impl AsyncByteSource for SdkContentStream {
    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        self.stream
            .next()
            .await
            .map(|item| item.map_err(|error| source_failure(&error)))
    }

    // None falls through to close(), whose Absent answer is the same Ok(()) a returned result would be
    #[cfg_attr(test, mutants::skip)]
    async fn aclose(&mut self) -> Option<CloseResult> {
        None
    }

    fn close(&mut self) -> CloseCall<'_> {
        CloseCall::Absent
    }
}

// httpx's iter_bytes raises the SDK's own error, which LiteLLM logs as a plain exception
pub(super) fn source_failure(error: &FilesError) -> SourceFailure {
    SourceFailure::Exception(ErrorClass::Unclassified(Box::new(ErrorContext {
        message: error.to_string(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    })))
}
