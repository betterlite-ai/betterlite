use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::FileContentRequest;
use betterlite_openai::files::stream::{
    FileContentStream as SdkFileStream, FileContentStreamingResult,
};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;

use crate::api::file_stream_logging::source::{
    AsyncByteSource, CloseCall, CloseResult, SourceFailure, StreamSource,
};
use crate::api::file_stream_logging::{FileContentStreamingResponse, FileStreamSetup};
use crate::api::vector_store_http_write::service::ClientKind;

use super::error::FileContentError;
use super::plan::known_provider;
use super::service::{COMPATIBLE_PROVIDERS, Reply};
use super::target::{
    CallerClient, ContentMode, ContentOperation, ContentReply, ContentTarget, FileContentDispatch,
    StreamIdentity, SuppliedSdk,
};
use super::{ContentAnswer, ContentCall, ContentContext, FileContentStream, StreamedContent};

impl<Dispatch> FileContentStream<'_, Dispatch>
where
    Dispatch: FileContentDispatch,
{
    // file_content_streaming: the logging identity is written before the provider branch, and the
    // returned iterator is wrapped by B1080
    pub(super) async fn streamed<'call>(
        &self,
        call: &ContentCall<'call>,
        provider: Option<String>,
        mode: ContentMode,
        request: FileContentRequest,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentAnswer, Dispatch> {
        let ContentContext {
            trusted,
            client,
            logger,
            logging,
        } = context;
        let logging = match logging {
            Some(logging) => logging,
            None => self
                .dispatch
                .create_logging(&self.new_logger(call.kwargs, mode)),
        };
        let provider_name = provider.clone().unwrap_or_else(|| String::from("None"));
        logging.record_identity(&StreamIdentity {
            model: normalized_model(call.model),
            custom_llm_provider: &provider_name,
            api_base: call.kwargs.api_base.as_deref(),
        });
        let name = provider.as_deref().unwrap_or_default();
        let target = if COMPATIBLE_PROVIDERS.contains(&name) {
            let supplied = match client {
                Some(CallerClient::Sdk(SuppliedSdk::OpenAi)) => Some(SuppliedSdk::OpenAi),
                Some(CallerClient::Sdk(SuppliedSdk::Azure) | CallerClient::Http(_)) | None => None,
            };
            ContentTarget::Compatible(self.plan(call.kwargs, mode).compatible(supplied))
        } else if name == "vertex_ai" {
            if !mode.is_async {
                return Err(FileContentError::SyncVertexStreaming);
            }
            let supplied = match client {
                Some(CallerClient::Http(handler)) if handler.kind == ClientKind::Asynchronous => {
                    Some(handler)
                }
                Some(CallerClient::Http(_) | CallerClient::Sdk(_)) | None => None,
            };
            let id = known_provider::<Dispatch::Error>(provider.as_deref())?;
            let Some(config) = self.dispatch.files_config(id) else {
                return Err(FileContentError::UnsupportedStreaming {
                    provider: provider_name,
                });
            };
            ContentTarget::Generic(Box::new(self.generic(
                call,
                call.extra_headers.clone().unwrap_or_default(),
                config,
                mode,
                trusted,
                supplied,
                logger,
            )))
        } else {
            return Err(FileContentError::UnsupportedStreaming {
                provider: name.to_owned(),
            });
        };
        let reply = self
            .dispatch
            .content(
                target,
                ContentOperation::Streamed {
                    request: &request,
                    chunk_size: stream_chunk_size(call.chunk_size),
                },
            )
            .await
            .map_err(FileContentError::Dispatch)?;
        let ContentReply::Streamed(result) = reply else {
            return Err(FileContentError::Reply);
        };
        let FileContentStreamingResult {
            stream,
            headers,
            exit,
        } = *result;
        let response = FileContentStreamingResponse::new(
            FileStreamSetup {
                source: StreamSource::Async(Box::new(SdkContentStream { stream })),
                file_id: call.file_id.to_owned(),
                model: call.model.map(str::to_owned),
                custom_llm_provider: provider,
                logger: Some(logging.stream_logger()),
            },
            Arc::new(ContentClock(self.clock)),
        );
        Ok(ContentAnswer::Streamed(Box::new(StreamedContent {
            response,
            headers,
            exit,
        })))
    }
}

// httpx iter_bytes(chunk_size=0) never advances, so a zero or unrepresentable size takes the SDK default
fn stream_chunk_size(chunk_size: u64) -> Option<NonZeroUsize> {
    usize::try_from(chunk_size).ok().and_then(NonZeroUsize::new)
}

// model or "", the value the logging object's model and its model_call_details["model"] both receive
fn normalized_model(model: Option<&str>) -> &str {
    model.filter(|name| !name.is_empty()).unwrap_or_default()
}

struct ContentClock(fn() -> DateTime<FixedOffset>);

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
            .map(|item| item.map_err(source_failure))
    }

    async fn aclose(&mut self) -> Option<CloseResult> {
        None
    }

    fn close(&mut self) -> CloseCall<'_> {
        CloseCall::Absent
    }
}

// httpx's iter_bytes raises the SDK's own error, which LiteLLM logs as a plain exception
pub(super) fn source_failure(error: FilesError) -> SourceFailure {
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
