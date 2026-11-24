use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::FileContentRequest;
use betterlite_openai::files::stream::{
    FileContentStream as SdkFileStream, FileContentStreamingResult,
};
use betterlite_openai::xai_batches::records::is_xai_batch_results_id;
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;

use crate::api::file_http_content::config::ContentParams;
use crate::api::file_stream_logging::handler::should_stream_file_content;
use crate::api::file_stream_logging::source::{
    AsyncByteSource, CloseCall, CloseResult, SourceFailure, StreamSource,
};
use crate::api::file_stream_logging::{FileContentStreamingResponse, FileStreamSetup};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

use super::error::FileContentError;
use super::plan::{Plan, content_timeout, known_provider, resolved_provider};
use super::target::{
    AzureContent, CallerClient, CompatibleContent, ContentLogger, ContentMode, ContentOperation,
    ContentReply, ContentTarget, FileContentDispatch, GenericContent, NewLogger, StreamIdentity,
    SuppliedSdk, XaiBatchContent,
};
use super::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
    StreamedContent,
};

// OPENAI_COMPATIBLE_BATCH_AND_FILES_PROVIDERS: these names share the OpenAI files client
const COMPATIBLE_PROVIDERS: [&str; 3] = ["openai", "hosted_vllm", "litellm_proxy"];

type Reply<Value, Dispatch> =
    Result<Value, FileContentError<<Dispatch as FileContentDispatch>::Error>>;

impl<Dispatch> FileContentStream<'_, Dispatch>
where
    Dispatch: FileContentDispatch,
{
    // file_content, with afile_content as its asynchronous entry: the timeout, the model hint, then the
    // streaming predicate before any provider branch
    pub async fn run<'call>(
        &self,
        entry: ContentEntry,
        call: &ContentCall<'call>,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentAnswer, Dispatch> {
        let provider = resolved_provider(entry, call, self.resolution);
        let mode = ContentMode {
            is_async: entry == ContentEntry::Async,
            timeout: content_timeout::<Dispatch::Error>(call.kwargs, provider.as_deref())?,
        };
        let request = content_request(call);
        if call.stream && should_stream_file_content(provider.as_deref()) {
            return self.streamed(call, provider, mode, request, context).await;
        }
        self.buffered(call, provider, mode, request, context).await
    }

    async fn buffered<'call>(
        &self,
        call: &ContentCall<'call>,
        provider: Option<String>,
        mode: ContentMode,
        request: FileContentRequest,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentAnswer, Dispatch> {
        let target = if provider.as_deref() == Some("xai") && is_xai_batch_results_id(call.file_id)
        {
            ContentTarget::XaiBatch(XaiBatchContent {
                batch_id: call.file_id,
                api_base: call.kwargs.api_base.as_deref(),
                api_key: call.kwargs.api_key.as_ref(),
                timeout: mode.timeout,
            })
        } else {
            let id = known_provider::<Dispatch::Error>(provider.as_deref())?;
            match self.dispatch.files_config(id) {
                Some(config) => {
                    let ContentContext {
                        trusted,
                        client,
                        logger,
                        logging: _logging,
                    } = context;
                    let handler = match client {
                        Some(CallerClient::Http(supplied)) => Some(supplied),
                        Some(CallerClient::Sdk(SuppliedSdk::Azure | SuppliedSdk::OpenAi))
                        | None => None,
                    };
                    ContentTarget::Generic(Box::new(self.generic(
                        call,
                        call.extra_headers.clone().unwrap_or_default(),
                        config,
                        mode,
                        trusted,
                        handler,
                        logger,
                    )))
                }
                None => {
                    let name = provider.as_deref().unwrap_or_default();
                    if COMPATIBLE_PROVIDERS.contains(&name) {
                        ContentTarget::Compatible(self.plan(call.kwargs, mode).compatible(None))
                    } else if name == "azure" {
                        let supplied = match context.client {
                            Some(CallerClient::Sdk(kind)) => Some(kind),
                            Some(CallerClient::Http(_)) | None => None,
                        };
                        ContentTarget::Azure(self.plan(call.kwargs, mode).azure(supplied))
                    } else {
                        return Err(FileContentError::Unsupported {
                            provider: name.to_owned(),
                        });
                    }
                }
            }
        };
        match self
            .dispatch
            .content(target, ContentOperation::Buffered { request: &request })
            .await
            .map_err(FileContentError::Dispatch)?
        {
            ContentReply::Buffered(content) => Ok(ContentAnswer::Buffered(content)),
            ContentReply::Streamed(_stream) => Err(FileContentError::Reply),
        }
    }

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

    fn generic<'call>(
        &self,
        call: &ContentCall<'call>,
        headers: BTreeMap<String, String>,
        config: Dispatch::Config,
        mode: ContentMode,
        trusted: Option<&'call TrustedCredentials>,
        client: Option<SuppliedClient<'call, Dispatch::Client>>,
        logger: Option<&'call mut LoggingState>,
    ) -> GenericContent<'call, Dispatch::Config, Dispatch::Client> {
        GenericContent {
            config,
            params: ContentParams {
                api_base: call.kwargs.api_base.clone(),
                api_key: call.kwargs.api_key.clone(),
            },
            trusted,
            headers,
            logger: match logger {
                Some(state) => ContentLogger::Supplied(state),
                None => ContentLogger::Created(Box::new(self.new_logger(call.kwargs, mode))),
            },
            client,
            mode,
        }
    }

    fn plan<'call>(&self, kwargs: &'call ContentKwargs, mode: ContentMode) -> Plan<'call> {
        Plan {
            kwargs,
            mode,
            settings: ResourceSdkSettings::from(&self.settings),
            options: ResourceSdkRequestOptions {
                api_base: kwargs.api_base.clone(),
                api_version: kwargs.api_version.clone(),
                api_key: kwargs.api_key.clone(),
                organization: kwargs.organization.clone(),
                vertex_project: None,
                vertex_location: None,
                vertex_credentials: None,
            },
        }
    }

    fn new_logger(&self, kwargs: &ContentKwargs, mode: ContentMode) -> NewLogger {
        self.plan(kwargs, mode)
            .new_logger((self.clock)(), (self.call_ids)().to_string())
    }
}

// LiteLLM's FileContentRequest: the timeout stays out of it, since every handler passes its own deadline
fn content_request(call: &ContentCall<'_>) -> FileContentRequest {
    FileContentRequest {
        file_id: call.file_id.to_owned(),
        extra_headers: call
            .extra_headers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        extra_body: call
            .extra_body
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        timeout: None,
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
