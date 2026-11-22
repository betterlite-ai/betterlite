use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation as AzureOperation, Reply, call};
use betterlite_config::sections::anthropic_credentials::Settings as AnthropicCredentials;
use betterlite_core::api::file_http_metadata::config::FileParams;
use betterlite_core::api::file_http_metadata::error::MetadataError;
use betterlite_core::api::file_http_metadata::service::{
    ListCall as MetadataListCall, MetadataCall, MetadataDeps, MetadataTarget, MetadataValue,
    RetrieveCall, prepare, send,
};
use betterlite_core::api::file_provider_read::target::{
    AzureRead, CompatibleRead, FileListing, FileReadDispatch, GenericRead, ReadAnswer, ReadLogger,
    ReadMode, ReadOperation, ReadTarget,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_core::api::vector_store_http_write::service::{
    ClientKind, LogContext, SuppliedClient,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{FileObject, FilePage, FileQuery};
use betterlite_openai::files::{
    ClientArgs, DEFAULT_TIMEOUT, FilesApi, FilesClient, get_openai_client, select_client,
};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};
use chrono::Utc;

use super::fixtures::azure::open;
use super::fixtures::logging::{Fixture, FixtureError};
use super::fixtures::metadata::{AnthropicFiles, anthropic_settings, metadata_params};
use super::fixtures::transport::{DispatchSource, WireClient};

#[derive(Debug, thiserror::Error)]
pub(super) enum ReadWireError {
    #[error(transparent)]
    OpenAi(#[from] FilesError),
    #[error(transparent)]
    Azure(#[from] AzureFilesError),
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error("the oracle sends no {config} request")]
    Unbuilt { config: &'static str },
    #[error("the file call answered with another reply kind")]
    Reply,
}

// the HTTPHandler a case hands over: its own httpx client adds these headers to every request it sends
pub(super) struct Handler {
    pub(super) headers: BTreeMap<String, String>,
}

pub(super) struct HttpDispatch {
    pub(super) openai: FilesApi<ProxiedClient>,
    pub(super) client: ProxiedClient,
    pub(super) credentials: AnthropicCredentials,
    pub(super) environment: BTreeMap<String, String>,
}

type Generic<'call> = GenericRead<'call, FileProviderConfig, Handler>;
type Target<'call> = ReadTarget<'call, FileProviderConfig, Handler>;

impl FileReadDispatch for HttpDispatch {
    type Config = FileProviderConfig;
    type Client = Handler;
    type Error = ReadWireError;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config> {
        file_config_selection("", provider)
    }

    async fn read(
        &self,
        target: Target<'_>,
        operation: ReadOperation<'_>,
    ) -> Result<ReadAnswer, Self::Error> {
        match target {
            ReadTarget::Compatible(read) => compatible_answer(&self.openai, &read, operation).await,
            ReadTarget::Azure(read) => {
                azure_answer(&self.client, &self.environment, &read, operation).await
            }
            ReadTarget::Generic(read) => {
                metadata_answer(*read, operation, &self.credentials, &self.client).await
            }
        }
    }
}

// the provider config's own call, run through B2462's prepare/send: client selection, the pre-call and the per-operation timeout rules
async fn metadata_answer(
    mut read: Generic<'_>,
    operation: ReadOperation<'_>,
    credentials: &AnthropicCredentials,
    client: &ProxiedClient,
) -> Result<ReadAnswer, ReadWireError> {
    let settings = anthropic_settings(credentials);
    let kind = client_kind(read.mode);
    let params = metadata_params(read.api_key.as_ref(), read.api_base.as_deref());
    let asked = metadata_call(operation, &params, &read.headers, read.mode);
    let source = DispatchSource {
        client: client.clone(),
    };
    let lent = lent_client(&read, kind, client);
    let config = provider_config(read.config, &settings)?;
    let supplied = lent.as_ref().map(|client| SuppliedClient {
        kind,
        transport: client,
    });
    let deps = MetadataDeps {
        config: &config,
        source: &source,
        supplied,
    };
    let fixture = Fixture::new()?;
    let mut built;
    let state: &mut LoggingState = match &mut read.logger {
        ReadLogger::Supplied(state) => state,
        ReadLogger::Created(new) => {
            built = fixture.state(new)?;
            &mut built
        }
    };
    let env = fixture.environment();
    let mut log = LogContext { state, env: &env };
    let prepared = prepare(&deps, kind, &asked, &mut log)?;
    answer(send(prepared, &mut log).await?)
}

// the config LiteLLM's provider lookup hands the metadata handler for this branch
fn provider_config(
    config: FileProviderConfig,
    settings: &AnthropicSettings,
) -> Result<AnthropicFiles<'_>, ReadWireError> {
    match config {
        FileProviderConfig::Anthropic => Ok(AnthropicFiles {
            settings,
            clock: || Utc::now().timestamp(),
        }),
        other @ (FileProviderConfig::GoogleAiStudio
        | FileProviderConfig::VertexAi
        | FileProviderConfig::Bedrock
        | FileProviderConfig::Manus
        | FileProviderConfig::Mistral
        | FileProviderConfig::Xai) => Err(ReadWireError::Unbuilt {
            config: other.class_name(),
        }),
    }
}

// the OpenAI files client both operations share, whose page or object comes back as the SDK parsed it
async fn compatible_answer(
    openai: &FilesApi<ProxiedClient>,
    read: &CompatibleRead,
    operation: ReadOperation<'_>,
) -> Result<ReadAnswer, ReadWireError> {
    let client = compatible_client(read)?;
    match operation {
        ReadOperation::Retrieve { file_id } => {
            let object = openai.retrieve_file(&client, file_id).await?;
            Ok(ReadAnswer::File(Box::new(object)))
        }
        ReadOperation::List { purpose } => {
            let query = FileQuery {
                purpose: purpose.map(str::to_owned),
            };
            let page = openai.list_files(&client, &query).await?;
            Ok(ReadAnswer::Listing(FileListing::Page(Box::new(page))))
        }
    }
}

// the azure call both operations share, whose reply kind must be the one the operation asks for
async fn azure_answer(
    client: &ProxiedClient,
    environment: &BTreeMap<String, String>,
    read: &AzureRead,
    operation: ReadOperation<'_>,
) -> Result<ReadAnswer, ReadWireError> {
    let opened = open(read, environment)?;
    match call(&opened, client, azure_operation(operation)).await? {
        Reply::Object(object) => azure_file(operation, object),
        Reply::Page(page) => azure_page(operation, page),
        Reply::Content(_) | Reply::Deleted(_) => Err(ReadWireError::Reply),
    }
}

fn metadata_call<'call>(
    operation: ReadOperation<'call>,
    params: &'call FileParams,
    headers: &'call BTreeMap<String, String>,
    mode: ReadMode,
) -> MetadataCall<'call> {
    let target = MetadataTarget {
        litellm_params: params,
        extra_headers: Some(headers),
        ssl_verify: None,
        timeout: Some(deadline(mode)),
    };
    match operation {
        ReadOperation::Retrieve { file_id } => {
            MetadataCall::Retrieve(RetrieveCall { file_id, target })
        }
        ReadOperation::List { purpose } => MetadataCall::List(MetadataListCall { purpose, target }),
    }
}

// a supplied HTTPHandler is selected only for its own mode, and then its client's headers ride on the request
fn lent_client(read: &Generic<'_>, kind: ClientKind, client: &ProxiedClient) -> Option<WireClient> {
    let supplied = read
        .client
        .as_ref()
        .filter(|supplied| supplied.kind == kind)?;
    Some(WireClient {
        client: client.clone(),
        headers: supplied.transport.headers.clone(),
    })
}

const fn client_kind(mode: ReadMode) -> ClientKind {
    if mode.is_async {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}

fn answer(value: MetadataValue) -> Result<ReadAnswer, ReadWireError> {
    match value {
        MetadataValue::File(file) => Ok(ReadAnswer::File(Box::new(file))),
        MetadataValue::Listed(files) => Ok(ReadAnswer::Listing(FileListing::Files(files))),
        MetadataValue::Deleted(_) => Err(ReadWireError::Reply),
    }
}

// the azure SDK call each read operation asks for
const fn azure_operation(operation: ReadOperation<'_>) -> AzureOperation<'_> {
    match operation {
        ReadOperation::Retrieve { file_id } => AzureOperation::Retrieve(file_id),
        ReadOperation::List { purpose } => AzureOperation::List(purpose),
    }
}

// a reply carrying one file, which only a retrieve operation asked for
fn azure_file(
    operation: ReadOperation<'_>,
    object: FileObject,
) -> Result<ReadAnswer, ReadWireError> {
    match operation {
        ReadOperation::Retrieve { .. } => Ok(ReadAnswer::File(Box::new(object))),
        ReadOperation::List { .. } => Err(ReadWireError::Reply),
    }
}

// a reply carrying one page, which only a list operation asked for
fn azure_page(operation: ReadOperation<'_>, page: FilePage) -> Result<ReadAnswer, ReadWireError> {
    match operation {
        ReadOperation::List { .. } => Ok(ReadAnswer::Listing(FileListing::Page(Box::new(page)))),
        ReadOperation::Retrieve { .. } => Err(ReadWireError::Reply),
    }
}

fn compatible_client(read: &CompatibleRead) -> Result<FilesClient, FilesError> {
    let args = ClientArgs {
        api_key: read.api_key.clone(),
        api_base: Some(read.api_base.clone()),
        timeout: Some(deadline(read.mode)),
        max_retries: read
            .max_retries
            .and_then(|retries| u32::try_from(retries).ok()),
        organization: read.organization.clone(),
    };
    select_client(
        get_openai_client(args, None, read.mode.is_async)?,
        read.mode.is_async,
    )
}

// a structured timeout only shapes the SDK's pool, so the wire deadline is its default
fn deadline(mode: ReadMode) -> Duration {
    match mode.timeout {
        FineTuningTimeout::Seconds(seconds) => {
            Duration::try_from_secs_f64(seconds).unwrap_or(DEFAULT_TIMEOUT)
        }
        FineTuningTimeout::Structured(_) => DEFAULT_TIMEOUT,
    }
}
