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
    CallerClient, ContentLogger, ContentMode, ContentOperation, ContentReply, ContentTarget,
    FileContentDispatch, GenericContent, NewLogger, SuppliedSdk, XaiBatchContent,
};
use super::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
};

// OPENAI_COMPATIBLE_BATCH_AND_FILES_PROVIDERS: these names share the OpenAI files client
pub(super) const COMPATIBLE_PROVIDERS: [&str; 3] = ["openai", "hosted_vllm", "litellm_proxy"];

// The two entry points settle the same values before they split; selection carries them across
pub(super) struct Selection {
    pub provider: Option<String>,
    pub mode: ContentMode,
    pub request: FileContentRequest,
}

// What only the server hands a generic handler: the trusted snapshot, the caller's HTTP handler and its logger
pub(super) struct ContentInputs<'call, Client> {
    pub trusted: Option<&'call TrustedCredentials>,
    pub client: Option<SuppliedClient<'call, Client>>,
    pub logger: Option<&'call mut LoggingState>,
}

// The two inputs streaming still needs for a generic target after it filters the caller's client itself
pub(super) struct ContentCredentials<'call> {
    pub trusted: Option<&'call TrustedCredentials>,
    pub logger: Option<&'call mut LoggingState>,
}

pub(super) type Reply<Value, Dispatch> =
    Result<Value, FileContentError<<Dispatch as FileContentDispatch>::Error>>;

impl<Dispatch> FileContentStream<'_, Dispatch>
where
    Dispatch: FileContentDispatch,
{
    // file_content and afile_content: timeout, model hint, streaming predicate, then the provider branch
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
        let selection = Selection {
            provider,
            mode,
            request: content_request(call),
        };
        if call.stream && should_stream_file_content(selection.provider.as_deref()) {
            return self.streamed(call, &selection, context).await;
        }
        self.buffered(call, &selection, context).await
    }

    async fn buffered<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentAnswer, Dispatch> {
        let target = self.target(call, selection, context)?;
        match self
            .dispatch
            .content(
                target,
                ContentOperation::Buffered {
                    request: &selection.request,
                },
            )
            .await
            .map_err(FileContentError::Dispatch)?
        {
            ContentReply::Buffered(content) => Ok(ContentAnswer::Buffered(content)),
            ContentReply::Streamed(_stream) => Err(FileContentError::Reply),
        }
    }

    // file_content's provider branch: the xai batch shortcut, then the provider's config, then the named fallbacks
    fn target<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentTarget<'call, Dispatch::Config, Dispatch::Client>, Dispatch> {
        if selection.provider.as_deref() == Some("xai") && is_xai_batch_results_id(call.file_id) {
            return Ok(ContentTarget::XaiBatch(XaiBatchContent {
                batch_id: call.file_id,
                api_base: call.kwargs.api_base.as_deref(),
                api_key: call.kwargs.api_key.as_ref(),
                timeout: selection.mode.timeout,
            }));
        }
        let id = known_provider::<Dispatch::Error>(selection.provider.as_deref())?;
        match self.dispatch.files_config(id) {
            Some(config) => Ok(self.configured(call, selection, config, context)),
            None => self.named(call, selection, context),
        }
    }

    // a provider config keeps only an HTTPHandler: neither SDK client class reaches the generic handler
    fn configured<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        config: Dispatch::Config,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> ContentTarget<'call, Dispatch::Config, Dispatch::Client> {
        let ContentContext {
            trusted,
            client: caller_client,
            logger,
            logging: _logging,
        } = context;
        let client = match caller_client {
            Some(CallerClient::Http(supplied)) => Some(supplied),
            Some(CallerClient::Sdk(SuppliedSdk::Azure | SuppliedSdk::OpenAi)) | None => None,
        };
        ContentTarget::Generic(Box::new(self.generic(
            call,
            selection,
            config,
            ContentInputs {
                trusted,
                client,
                logger,
            },
        )))
    }

    // without a provider config: the OpenAI-compatible names, then azure, then the 400 LiteLLM raises
    fn named<'call>(
        &self,
        call: &ContentCall<'call>,
        selection: &Selection,
        context: ContentContext<'call, Dispatch::Client, Dispatch::Logging>,
    ) -> Reply<ContentTarget<'call, Dispatch::Config, Dispatch::Client>, Dispatch> {
        let name = selection.provider.as_deref().unwrap_or_default();
        if COMPATIBLE_PROVIDERS.contains(&name) {
            return Ok(ContentTarget::Compatible(
                self.plan(call.kwargs, selection.mode).compatible(None),
            ));
        }
        if name == "azure" {
            let supplied = match context.client {
                Some(CallerClient::Sdk(kind)) => Some(kind),
                Some(CallerClient::Http(_)) | None => None,
            };
            return Ok(ContentTarget::Azure(
                self.plan(call.kwargs, selection.mode).azure(supplied),
            ));
        }
        Err(FileContentError::Unsupported {
            provider: name.to_owned(),
        })
    }

    pub(super) fn generic<'call>(
        &self,
        call: &ContentCall<'_>,
        selection: &Selection,
        config: Dispatch::Config,
        inputs: ContentInputs<'call, Dispatch::Client>,
    ) -> GenericContent<'call, Dispatch::Config, Dispatch::Client> {
        let ContentInputs {
            trusted,
            client,
            logger,
        } = inputs;
        GenericContent {
            config,
            params: ContentParams {
                api_base: call.kwargs.api_base.clone(),
                api_key: call.kwargs.api_key.clone(),
            },
            trusted,
            headers: call.extra_headers.clone().unwrap_or_default(),
            logger: logger.map_or_else(
                || ContentLogger::Created(Box::new(self.new_logger(call.kwargs, selection.mode))),
                ContentLogger::Supplied,
            ),
            client,
            mode: selection.mode,
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
