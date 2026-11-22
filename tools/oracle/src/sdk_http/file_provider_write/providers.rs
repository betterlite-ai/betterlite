use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::files::api::FilesApi as AnthropicFiles;
use betterlite_anthropic::files::error::FilesError as AnthropicError;
use betterlite_anthropic::files::request::{FileCall, FilesClient as AnthropicClient, Upload};
use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_write::{
    ConfigDelete, ConfigUpload, DeleteTarget, FileWriteProviders, ProviderCall, UploadTarget,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{FileCreate, FileDeleted, FileObject};
use betterlite_openai::files::{
    ClientArgs, DEFAULT_TIMEOUT, FilesApi, FilesClient, get_openai_client, select_client,
};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};
use chrono::Utc;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub(super) enum WireError {
    #[error(transparent)]
    OpenAi(#[from] FilesError),
    #[error(transparent)]
    Anthropic(#[from] AnthropicError),
    #[error("the oracle drives no {0} upload or delete")]
    NotWired(&'static str),
    #[error("the extra header {0} is not a valid HTTP header")]
    Header(String),
}
// the provider SDK clients LiteLLM's branches end in; Azure and the configs other than Anthropic are proven in Rust only
pub(super) struct OracleProviders {
    openai: FilesApi<ProxiedClient>,
    anthropic: AnthropicFiles<ProxiedClient>,
}
impl OracleProviders {
    pub(super) fn new(transport: ProxiedClient) -> Self {
        Self {
            openai: FilesApi {
                transport: transport.clone(),
                boundaries: Uuid::new_v4,
            },
            anthropic: AnthropicFiles {
                transport,
                clock: || Utc::now().timestamp(),
            },
        }
    }

    async fn anthropic_upload(
        &self,
        upload: &ConfigUpload<'_, Self>,
    ) -> Result<FileObject, WireError> {
        let (client, headers) = anthropic_transport(
            upload.config,
            upload.api_key,
            upload.api_base,
            upload.timeout,
            &upload.headers,
        )?;
        let file = FileData::Tuple3 {
            filename: upload.data.file.filename.clone(),
            content: FileBody::Bytes(upload.data.file.data.clone()),
            content_type: upload.data.file.content_type.clone(),
        };
        let call = Upload {
            file: Some(&file),
            purpose: Some(upload.data.purpose.wire_name()),
            boundary: Uuid::new_v4().simple().to_string(),
            headers: &headers,
        };
        Ok(self.anthropic.create_file(&client, &call).await?)
    }

    async fn openai_upload(
        &self,
        call: &ProviderCall,
        data: &FileCreate,
    ) -> Result<FileObject, WireError> {
        let client = openai_client(call)?;
        self.openai
            .create_file(&client, data)
            .await
            .map_err(WireError::from)
    }
}
impl FileWriteProviders for OracleProviders {
    type Config = FileProviderConfig;
    type Logger = ();
    type Client = ();
    type Error = WireError;

    fn files_config(&self, provider: ProviderId) -> Option<FileProviderConfig> {
        file_config_selection("", provider)
    }

    async fn upload(&self, target: UploadTarget<'_, Self>) -> Result<FileObject, WireError> {
        match target {
            UploadTarget::Config(upload) => self.anthropic_upload(&upload).await,
            UploadTarget::OpenAi { call, data } => self.openai_upload(&call, &data).await,
            UploadTarget::Azure { .. } => Err(WireError::NotWired("AzureOpenAIFilesAPI")),
        }
    }

    async fn delete(&self, target: DeleteTarget<'_, Self>) -> Result<FileDeleted, WireError> {
        match target {
            DeleteTarget::Config(delete) => {
                let (client, headers) = anthropic_transport(
                    delete.config,
                    delete.params.api_key,
                    delete.params.api_base,
                    delete.timeout,
                    &delete.headers,
                )?;
                let call = FileCall {
                    file_id: delete.file_id,
                    headers: &headers,
                };
                Ok(self.anthropic.delete_file(&client, &call).await?)
            }
            DeleteTarget::OpenAi { call, file_id } => {
                let client = openai_client(&call)?;
                Ok(self.openai.delete_file(&client, file_id).await?)
            }
            DeleteTarget::Azure { .. } => Err(WireError::NotWired("AzureOpenAIFilesAPI")),
        }
    }
}
// get_openai_client then the files wrapper's own client check, as OpenAIFilesAPI makes them
fn openai_client(call: &ProviderCall) -> Result<FilesClient, FilesError> {
    let is_async = call.mode == Mode::Asynchronous;
    let args = ClientArgs {
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        timeout: Some(deadline(call.timeout)),
        max_retries: call.max_retries.and_then(|count| u32::try_from(count).ok()),
        organization: call.organization.clone(),
    };
    select_client(get_openai_client(args, None, is_async)?, is_async)
}
// the client and headers both generic file calls reach the Anthropic SDK with
fn anthropic_transport(
    config: FileProviderConfig,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    timeout: FineTuningTimeout,
    headers: &IndexMap<String, String>,
) -> Result<(AnthropicClient, HeaderMap), WireError> {
    let client = match config {
        FileProviderConfig::Anthropic => AnthropicClient {
            api_key: api_key.cloned(),
            api_base: api_base.map(str::to_owned),
            settings: AnthropicSettings::default(),
            timeout: deadline(timeout),
        },
        FileProviderConfig::GoogleAiStudio
        | FileProviderConfig::VertexAi
        | FileProviderConfig::Bedrock
        | FileProviderConfig::Manus
        | FileProviderConfig::Mistral
        | FileProviderConfig::Xai => return Err(WireError::NotWired(config.class_name())),
    };
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let parsed_name = HeaderName::try_from(name.as_str())
            .map_err(|_invalid| WireError::Header(name.clone()))?;
        let parsed_value = HeaderValue::try_from(value.as_str())
            .map_err(|_invalid| WireError::Header(name.clone()))?;
        map.insert(parsed_name, parsed_value);
    }
    Ok((client, map))
}
// an httpx.Timeout bounds the whole request by its read phase here, as the SDK's read deadline does
fn deadline(timeout: FineTuningTimeout) -> Duration {
    let seconds = match timeout {
        FineTuningTimeout::Seconds(seconds) => Some(seconds),
        FineTuningTimeout::Structured(phases) => phases.read,
    };
    seconds
        .and_then(|value| Duration::try_from_secs_f64(value).ok())
        .filter(|duration| !duration.is_zero())
        .unwrap_or(DEFAULT_TIMEOUT)
}
