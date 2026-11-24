use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation as AzureOperation, Reply, call};
use betterlite_core::api::file_content_stream::target::{
    AzureContent, CompatibleContent, ContentLogger, ContentMode, ContentOperation, ContentReply,
    ContentTarget, FileContentDispatch, GenericContent, NewLogger, SuppliedSdk,
};
use betterlite_core::api::file_http_content::error::ContentError;
use betterlite_core::api::file_http_content::service::buffered::retrieve_file_content;
use betterlite_core::api::file_http_content::service::streaming::retrieve_content_streaming;
use betterlite_core::api::file_http_content::service::{
    ClientSource, ContentDeps, LogContext, RetrieveCall, prepare,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::stream::DEFAULT_CHUNK_SIZE;
use betterlite_openai::files::{
    ClientArgs, FilesApi, FilesClient, get_openai_client, select_client,
};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::fixtures::azure::open;
use super::fixtures::content::AnthropicContent;
use super::fixtures::logging::{Fixture, RecordingLogging};
use super::fixtures::transport::ContentSource;

// the key and base path a case supplies with its own OpenAI client
const SUPPLIED_KEY: &str = "sk-supplied";
const SUPPLIED_SUFFIX: &str = "supplied";

#[derive(Debug, thiserror::Error)]
pub(super) enum ContentWireError {
    #[error(transparent)]
    OpenAi(#[from] FilesError),
    #[error(transparent)]
    Azure(#[from] AzureFilesError),
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error(transparent)]
    Fixture(#[from] SdkHttpError),
    #[error("the oracle sends no {config} content request")]
    Unbuilt { config: &'static str },
    #[error("the file content call answered with another reply kind")]
    Reply,
}

// the HTTPHandler a case hands over: its own httpx client adds these headers to every request it sends
pub(super) struct Handler {
    pub(super) headers: BTreeMap<String, String>,
}

pub(super) struct ContentDispatch {
    pub(super) openai: FilesApi<ProxiedClient>,
    pub(super) client: ProxiedClient,
    pub(super) anthropic: AnthropicSettings,
    pub(super) environment: BTreeMap<String, String>,
}

type Generic<'call> = GenericContent<'call, FileProviderConfig, Handler>;
type Target<'call> = ContentTarget<'call, FileProviderConfig, Handler>;

impl FileContentDispatch for ContentDispatch {
    type Config = FileProviderConfig;
    type Client = Handler;
    type Logging = RecordingLogging;
    type Error = ContentWireError;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config> {
        file_config_selection("", provider)
    }

    fn create_logging(&self, _seed: &NewLogger) -> Arc<Self::Logging> {
        Arc::new(RecordingLogging::default())
    }

    async fn content(
        &self,
        target: Target<'_>,
        operation: ContentOperation<'_>,
    ) -> Result<ContentReply, Self::Error> {
        match target {
            ContentTarget::Compatible(content) => {
                compatible_answer(&self.openai, &content, operation).await
            }
            ContentTarget::Azure(azure) => {
                azure_answer(&self.client, &self.environment, &azure, operation).await
            }
            ContentTarget::Generic(generic) => {
                generic_answer(*generic, operation, &self.anthropic, &self.client).await
            }
            ContentTarget::XaiBatch(_) => Err(ContentWireError::Unbuilt {
                config: "xai batch results",
            }),
        }
    }
}

// the provider config's own content call, run through B2463's prepare and handlers
async fn generic_answer(
    generic: Generic<'_>,
    operation: ContentOperation<'_>,
    settings: &AnthropicSettings,
    client: &ProxiedClient,
) -> Result<ContentReply, ContentWireError> {
    let GenericContent {
        config,
        params,
        trusted: _trusted,
        headers,
        mut logger,
        client: supplied,
        mode,
    } = generic;
    let config = content_config(config, settings)?;
    let kind = client_kind(mode);
    let lent = supplied
        .filter(|supplied| supplied.kind == kind)
        .map(|SuppliedClient { transport, .. }| transport.headers.clone());
    let source = ContentSource {
        client: client.clone(),
        headers: lent.unwrap_or_default(),
    };
    let chunk_size = match &operation {
        ContentOperation::Buffered { .. } => None,
        ContentOperation::Streamed { chunk_size, .. } => *chunk_size,
    };
    let request = match &operation {
        ContentOperation::Buffered { request } | ContentOperation::Streamed { request, .. } => {
            *request
        }
    };
    let retrieve = RetrieveCall {
        request,
        params: &params,
        headers: &headers,
        timeout: deadline(mode),
    };
    let fixture = Fixture::new()?;
    let mut built;
    let state: &mut LoggingState = match &mut logger {
        ContentLogger::Supplied(state) => state,
        ContentLogger::Created(new) => {
            built = fixture.state(new)?;
            &mut built
        }
    };
    let env = fixture.environment();
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let prepared = {
        let mut log = LogContext {
            state: &mut *state,
            env: &env,
        };
        prepare(&deps, &retrieve, &mut log)?
    };
    match operation {
        ContentOperation::Buffered { .. } => Ok(ContentReply::Buffered(Box::new(
            retrieve_file_content(prepared, state).await?,
        ))),
        ContentOperation::Streamed { .. } => {
            let size = chunk_size
                .unwrap_or(NonZeroUsize::new(DEFAULT_CHUNK_SIZE).unwrap_or(NonZeroUsize::MIN));
            Ok(ContentReply::Streamed(Box::new(
                retrieve_content_streaming(prepared, size, state).await?,
            )))
        }
    }
}

// the config B2464's lookup hands the content handler for this branch
fn content_config<'settings>(
    config: FileProviderConfig,
    settings: &'settings AnthropicSettings,
) -> Result<AnthropicContent<'settings>, ContentWireError> {
    match config {
        FileProviderConfig::Anthropic => Ok(AnthropicContent { settings }),
        other @ (FileProviderConfig::GoogleAiStudio
        | FileProviderConfig::VertexAi
        | FileProviderConfig::Bedrock
        | FileProviderConfig::Manus
        | FileProviderConfig::Mistral
        | FileProviderConfig::Xai) => Err(ContentWireError::Unbuilt {
            config: other.class_name(),
        }),
    }
}

// the OpenAI files client the compatible branch shares; an SDK client the caller supplied stays its own
async fn compatible_answer(
    openai: &FilesApi<ProxiedClient>,
    content: &CompatibleContent,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, ContentWireError> {
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
    let (api_key, api_base, organization) = compatible_credentials(content);
    let args = ClientArgs {
        api_key,
        api_base: Some(api_base),
        timeout: deadline(content.mode),
        max_retries: content
            .max_retries
            .and_then(|retries| u32::try_from(retries).ok()),
        organization,
    };
    select_client(
        get_openai_client(args, None, content.mode.is_async)?,
        content.mode.is_async,
    )
}

// a supplied OpenAI client sends with its own base and key, as the case builds it, and the call's are unused
fn compatible_credentials(
    content: &CompatibleContent,
) -> (Option<SecretString>, String, Option<String>) {
    match content.client {
        Some(SuppliedSdk::OpenAi) => (
            Some(SecretString::from(SUPPLIED_KEY)),
            format!("{}/{SUPPLIED_SUFFIX}", content.api_base),
            None,
        ),
        Some(SuppliedSdk::Azure) | None => (
            content.api_key.clone(),
            content.api_base.clone(),
            content.organization.clone(),
        ),
    }
}

// the azure SDK call a buffered content request asks for, over the mock
async fn azure_answer(
    client: &ProxiedClient,
    environment: &BTreeMap<String, String>,
    content: &AzureContent,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, ContentWireError> {
    let opened = open(content, environment)?;
    match operation {
        ContentOperation::Buffered { request } => {
            match call(&opened, client, AzureOperation::Content(request)).await? {
                Reply::Content(content) => Ok(ContentReply::Buffered(Box::new(content))),
                Reply::Object(_) | Reply::Page(_) | Reply::Deleted(_) => {
                    Err(ContentWireError::Reply)
                }
            }
        }
        // azure is outside the streaming set, so file_content_streaming never selects it
        ContentOperation::Streamed { .. } => Err(ContentWireError::Reply),
    }
}

const fn client_kind(mode: ContentMode) -> ClientKind {
    if mode.is_async {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}

// a structured timeout shapes the client's pool rather than one deadline
fn deadline(mode: ContentMode) -> Option<Duration> {
    match mode.timeout {
        FineTuningTimeout::Seconds(seconds) => Duration::try_from_secs_f64(seconds).ok(),
        FineTuningTimeout::Structured(_) => None,
    }
}
