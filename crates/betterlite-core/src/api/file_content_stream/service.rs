use std::collections::BTreeMap;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_openai::files::records::FileContentRequest;
use betterlite_openai::xai_batches::records::is_xai_batch_results_id;

use crate::api::file_http_content::config::ContentParams;
use crate::api::file_stream_logging::handler::should_stream_file_content;
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use crate::api::vector_store_http_write::service::SuppliedClient;

use super::error::FileContentError;
use super::plan::{Plan, content_timeout, known_provider, resolved_provider};
use super::target::{
    AzureContent, CallerClient, CompatibleContent, ContentLogger, ContentMode, ContentOperation,
};
use super::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
};

// OPENAI_COMPATIBLE_BATCH_AND_FILES_PROVIDERS: these names share the OpenAI files client
pub(super) const COMPATIBLE_PROVIDERS: [&str; 3] = ["openai", "hosted_vllm", "litellm_proxy"];

pub(super) type Reply<Value, Dispatch> =
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
    pub(super) fn generic<'call>(
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

    pub(super) fn plan<'call>(
        &self,
        kwargs: &'call ContentKwargs,
        mode: ContentMode,
    ) -> Plan<'call> {
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

    pub(super) fn new_logger(&self, kwargs: &ContentKwargs, mode: ContentMode) -> NewLogger {
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
