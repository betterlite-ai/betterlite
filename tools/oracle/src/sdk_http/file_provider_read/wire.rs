use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_anthropic::files::api::FilesApi as AnthropicApi;
use betterlite_anthropic::files::error::FilesError as AnthropicError;
use betterlite_anthropic::files::request::{FileCall, FilesClient as AnthropicClient, ListCall};
use betterlite_azure::azure_files::client::auth::FileAuth;
use betterlite_azure::azure_files::client::{Opened, SelectedClient};
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation as AzureOperation, Reply, call};
use betterlite_azure::endpoints::projection::ClientKind as AzureKind;
use betterlite_config::sections::anthropic_credentials::Settings as AnthropicCredentials;
use betterlite_core::api::file_provider_read::target::{
    AzureRead, CompatibleRead, FileListing, FileReadDispatch, GenericRead, ReadAnswer, ReadMode,
    ReadOperation, ReadTarget,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{FileObject, FilePage, FileQuery};
use betterlite_openai::files::{
    ClientArgs, DEFAULT_TIMEOUT, FilesApi, FilesClient, get_openai_client, select_client,
};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub(super) enum ReadWireError {
    #[error(transparent)]
    OpenAi(#[from] FilesError),
    #[error(transparent)]
    Azure(#[from] AzureFilesError),
    #[error(transparent)]
    Anthropic(#[from] AnthropicError),
    #[error("the oracle sends no {config} request")]
    Unbuilt { config: &'static str },
    #[error("the azure files call answered with another reply kind")]
    Reply,
    #[error("the header {name} is not valid")]
    Header { name: String },
}

// the HTTPHandler a case hands over: its own httpx client adds these headers to every request it sends
pub(super) struct Handler {
    pub(super) headers: BTreeMap<String, String>,
}

pub(super) struct HttpDispatch {
    pub(super) openai: FilesApi<ProxiedClient>,
    pub(super) anthropic: AnthropicApi<ProxiedClient>,
    pub(super) credentials: AnthropicCredentials,
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
        match (target, operation) {
            (ReadTarget::Compatible(read), ReadOperation::Retrieve { file_id }) => {
                let client = compatible_client(&read)?;
                let object = self.openai.retrieve_file(&client, file_id).await?;
                Ok(ReadAnswer::File(Box::new(object)))
            }
            (ReadTarget::Compatible(read), ReadOperation::List { purpose }) => {
                let client = compatible_client(&read)?;
                let query = FileQuery {
                    purpose: purpose.map(str::to_owned),
                };
                let page = self.openai.list_files(&client, &query).await?;
                Ok(ReadAnswer::Listing(FileListing::Page(Box::new(page))))
            }
            (ReadTarget::Azure(read), asked) => self.azure_answer(&read, asked).await,
            (ReadTarget::Generic(read), ReadOperation::Retrieve { file_id }) => {
                let (client, headers) = self.anthropic_client(&read)?;
                let file = FileCall {
                    file_id,
                    headers: &headers,
                };
                let object = self.anthropic.retrieve_file(&client, &file).await?;
                Ok(ReadAnswer::File(Box::new(object)))
            }
            (ReadTarget::Generic(read), ReadOperation::List { purpose }) => {
                let (client, headers) = self.anthropic_client(&read)?;
                let listed = ListCall {
                    purpose,
                    headers: &headers,
                };
                let files = self.anthropic.list_files(&client, &listed).await?;
                Ok(ReadAnswer::Listing(FileListing::Files(files)))
            }
        }
    }
}

impl HttpDispatch {
    // the azure call both operations share, whose reply kind must be the one the operation asks for
    async fn azure_answer(
        &self,
        read: &AzureRead,
        operation: ReadOperation<'_>,
    ) -> Result<ReadAnswer, ReadWireError> {
        let asked = match operation {
            ReadOperation::Retrieve { file_id } => AzureOperation::Retrieve(file_id),
            ReadOperation::List { purpose } => AzureOperation::List(purpose),
        };
        let client = azure_client(read)?;
        match call(&client, &self.openai.transport, asked).await? {
            Reply::Object(object) => azure_file(operation, object),
            Reply::Page(page) => azure_page(operation, page),
            Reply::Content(_) | Reply::Deleted(_) => Err(ReadWireError::Reply),
        }
    }

    // only the anthropic config is sent; a supplied handler of the call's own mode lends its headers
    fn anthropic_client(
        &self,
        read: &Generic<'_>,
    ) -> Result<(AnthropicClient, HeaderMap), ReadWireError> {
        if read.config != FileProviderConfig::Anthropic {
            return Err(ReadWireError::Unbuilt {
                config: read.config.class_name(),
            });
        }
        let wanted = if read.mode.is_async {
            ClientKind::Asynchronous
        } else {
            ClientKind::Sync
        };
        let lent = read
            .client
            .as_ref()
            .filter(|supplied| supplied.kind == wanted)
            .map(|supplied| &supplied.transport.headers);
        let mut headers = HeaderMap::new();
        for (name, value) in lent.into_iter().flatten().chain(&read.headers) {
            let (key, parsed) = HeaderName::from_bytes(name.as_bytes())
                .ok()
                .zip(HeaderValue::from_str(value).ok())
                .ok_or_else(|| ReadWireError::Header { name: name.clone() })?;
            headers.insert(key, parsed);
        }
        let credentials = &self.credentials;
        let client = AnthropicClient {
            api_key: read.api_key.clone(),
            api_base: read.api_base.clone(),
            settings: AnthropicSettings {
                api_key: credentials.api_key.clone(),
                auth_token: credentials.auth_token.clone(),
                api_base: credentials.api_base.clone(),
                use_bearer_for_custom_base: credentials.use_bearer_for_custom_base,
                anthropic_version: ANTHROPIC_VERSION.to_owned(),
            },
            timeout: deadline(read.mode),
        };
        Ok((client, headers))
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

// the AzureOpenAI client a key and an endpoint build: <endpoint>/openai/ with the version in the query
fn azure_client(read: &AzureRead) -> Result<Opened, AzureFilesError> {
    let base = read
        .api_base
        .as_deref()
        .ok_or(AzureFilesError::MissingBaseUrl)?;
    let key = read
        .api_key
        .clone()
        .ok_or(AzureFilesError::MissingCredentials)?;
    let base_url = Url::parse(&format!("{}/openai/", base.trim_end_matches('/')))
        .map_err(|source| AzureFilesError::BaseUrl { source })?;
    let files = FilesClient {
        api_key: key.clone(),
        base_url,
        organization: None,
        timeout: deadline(read.mode),
        max_retries: 0,
    };
    let kind = if read.mode.is_async {
        AzureKind::AsyncAzureOpenAi
    } else {
        AzureKind::AzureOpenAi
    };
    let client = SelectedClient::new(
        files,
        &FileAuth::ApiKey(key),
        read.api_version.clone(),
        kind,
    )?;
    Ok(Opened {
        client,
        is_async: read.mode.is_async,
    })
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
