use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_azure::azure_files::client::{
    ClientMaterializer, ClientSettings, Connection, Opened, SelectedClient,
};
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation as AzureOperation, Reply as AzureReply, call};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    ClientIdentity, ClientKind, ClientOutcome, SessionKind,
};
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::NoSources;
use betterlite_core::api::file_content_stream::target::{
    AzureContent, CompatibleContent, ContentLogging, ContentOperation, ContentReply, ContentTarget,
    FileContentDispatch, NewLogger, StreamIdentity,
};
use betterlite_core::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::{
    ClientArgs, FilesApi, FilesClient, get_openai_client, select_client,
};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};
use chrono::{DateTime, FixedOffset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DispatchError {
    #[error(transparent)]
    Files(#[from] FilesError),
    #[error(transparent)]
    Azure(#[from] AzureFilesError),
    #[error("the {config} file content call is not implemented in this build")]
    Unbuilt { config: &'static str },
    #[error("the file content dispatch answered with another reply kind")]
    Reply,
}

pub(crate) struct ContentDispatch {
    pub(crate) openai: FilesApi<ProxiedClient>,
    pub(crate) client: ProxiedClient,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) endpoints: EndpointSettings,
}

impl FileContentDispatch for ContentDispatch {
    type Config = FileProviderConfig;
    type Client = ();
    type Logging = SilentLogging;
    type Error = DispatchError;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config> {
        file_config_selection("", provider)
    }

    fn create_logging(&self, _seed: &NewLogger) -> Arc<Self::Logging> {
        Arc::new(SilentLogging::default())
    }

    fn content(
        &self,
        target: ContentTarget<'_, FileProviderConfig, ()>,
        operation: ContentOperation<'_>,
    ) -> impl Future<Output = Result<ContentReply, DispatchError>> + Send {
        answer(self, target, operation)
    }
}

async fn answer(
    dispatch: &ContentDispatch,
    target: ContentTarget<'_, FileProviderConfig, ()>,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, DispatchError> {
    match target {
        ContentTarget::Compatible(content) => {
            compatible_answer(&dispatch.openai, &content, operation).await
        }
        ContentTarget::Azure(content) => azure_answer(dispatch, &content, operation).await,
        ContentTarget::Generic(content) => Err(DispatchError::Unbuilt {
            config: content.config.class_name(),
        }),
        ContentTarget::XaiBatch(_content) => Err(DispatchError::Unbuilt {
            config: "xai batch results",
        }),
    }
}

async fn compatible_answer(
    openai: &FilesApi<ProxiedClient>,
    content: &CompatibleContent,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, DispatchError> {
    let client = compatible_client(content)?;
    match operation {
        ContentOperation::Buffered { request } => Ok(ContentReply::Buffered(Box::new(
            openai.file_content(&client, request).await?,
        ))),
        ContentOperation::Streamed {
            request,
            chunk_size,
        } => Ok(ContentReply::Streamed(Box::new(
            openai
                .file_content_streaming(&client, request, chunk_size)
                .await?,
        ))),
    }
}

fn compatible_client(content: &CompatibleContent) -> Result<FilesClient, FilesError> {
    let args = ClientArgs {
        api_key: content.api_key.clone(),
        api_base: Some(content.api_base.clone()),
        timeout: deadline(content.mode.timeout),
        max_retries: content
            .max_retries
            .and_then(|retries| u32::try_from(retries).ok()),
        organization: content.organization.clone(),
    };
    select_client(
        get_openai_client(args, None, content.mode.is_async)?,
        content.mode.is_async,
    )
}

// Match the other SDK bridges: read, connect, write, pool, then the SDK default.
pub(super) fn deadline(timeout: FineTuningTimeout) -> Option<Duration> {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => Duration::try_from_secs_f64(seconds).ok(),
        FineTuningTimeout::Structured(phases) => {
            let seconds = phases
                .read
                .or(phases.connect)
                .or(phases.write)
                .or(phases.pool)
                .unwrap_or(600.0_f64);
            Duration::try_from_secs_f64(seconds).ok()
        }
    }
}

async fn azure_answer(
    dispatch: &ContentDispatch,
    content: &AzureContent,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, DispatchError> {
    let opened = open_azure(dispatch, content)?;
    // azure is outside the streaming set, so file_content_streaming never selects it
    let ContentOperation::Buffered { request } = operation else {
        return Err(DispatchError::Reply);
    };
    match call(&opened, &dispatch.client, AzureOperation::Content(request)).await? {
        AzureReply::Content(binary) => Ok(ContentReply::Buffered(Box::new(binary))),
        AzureReply::Object(_) | AzureReply::Page(_) | AzureReply::Deleted(_) => {
            Err(DispatchError::Reply)
        }
    }
}

fn open_azure(
    dispatch: &ContentDispatch,
    content: &AzureContent,
) -> Result<Opened, AzureFilesError> {
    let sources = NoSources;
    let materializer = NoMaterializer;
    let mut cache = NoCache;
    let mut refresh = RefreshProviderCache::new(&sources);
    Connection {
        settings: ClientSettings {
            api_key: content.api_key.clone(),
            api_base: content.api_base.clone(),
            api_version: content.api_version.clone(),
        },
        params: content.params.clone(),
        provider: None,
        supplied: None,
        is_async: content.mode.is_async,
        endpoints: &dispatch.endpoints,
        environment: &dispatch.environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    }
    .open()
}

// nothing is supplied or cached, so the azure handler always builds its client from its own arguments
struct NoCache;

impl ClientCache for NoCache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

struct NoMaterializer;

impl ClientMaterializer for NoMaterializer {
    fn materialize(
        &self,
        _outcome: &ClientOutcome<'_>,
    ) -> Result<Option<SelectedClient>, AzureFilesError> {
        Ok(None)
    }
}

// The logging B250 and B251 would supply. No production bridge exists in this build, so a stream logs to nothing.
#[derive(Default)]
pub(crate) struct SilentLogging {
    pub(crate) provider: OnceLock<String>,
}

impl ContentLogging for SilentLogging {
    fn record_identity(&self, identity: &StreamIdentity<'_>) {
        let _previous = self.provider.set(identity.custom_llm_provider.to_owned());
    }

    fn stream_logger(self: Arc<Self>) -> Arc<dyn FileStreamLogger> {
        self
    }
}

#[async_trait]
impl FileStreamLogger for SilentLogging {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    // nothing reads the api_base this logger could name: no logging bridge exists in this build
    #[cfg_attr(test, mutants::skip)]
    fn details_api_base(&self) -> Option<String> {
        None
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        None
    }

    fn success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }
}
