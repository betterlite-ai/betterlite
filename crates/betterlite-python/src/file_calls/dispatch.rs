use std::collections::BTreeMap;

use betterlite_azure::azure_files::client::Opened;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{
    Operation as AzureOperation, Reply as AzureReply, call as azure_call,
};
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_core::api::file_provider_read::target::{
    CompatibleRead, FileListing, FileReadDispatch, ReadAnswer, ReadOperation, ReadTarget,
};
use betterlite_core::api::file_provider_write::{
    DeleteTarget, FileWriteProviders, ProviderCall, UploadTarget,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::FilesApi;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{FileDeleted, FileObject, FileQuery};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};

use self::azure::{azure_answer, open_azure};
use self::clients::{ClientSource, is_asynchronous, litellm_params, openai_client, settings};

mod azure;
pub(super) mod clients;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DispatchError {
    #[error(transparent)]
    Files(#[from] FilesError),
    #[error(transparent)]
    Azure(#[from] AzureFilesError),
    #[error("the {config} file call is not implemented in this build")]
    Unbuilt { config: &'static str },
    #[error("the file call answered with another reply kind")]
    Reply,
}

pub(crate) struct FileCallDispatch {
    pub(crate) openai: FilesApi<ProxiedClient>,
    pub(crate) client: ProxiedClient,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) endpoints: EndpointSettings,
}

enum WriteCall<'call> {
    Upload(UploadTarget<'call, FileCallDispatch>),
    Delete(DeleteTarget<'call, FileCallDispatch>),
}

enum WriteReply {
    Object(FileObject),
    Deleted(FileDeleted),
}

impl FileCallDispatch {
    // the azure handler takes its client arguments the same way for upload and delete
    fn open_provider(
        &self,
        call: &ProviderCall,
        params: LiteLlmParams,
    ) -> Result<Opened, AzureFilesError> {
        open_azure(
            self,
            settings(
                call.api_key.clone(),
                call.api_base.clone(),
                call.api_version.clone(),
            ),
            is_asynchronous(call.mode),
            params,
        )
    }

    async fn send(&self, call: WriteCall<'_>) -> Result<WriteReply, DispatchError> {
        match call {
            WriteCall::Upload(UploadTarget::OpenAi { call, data }) => {
                let client = openai_client(ClientSource::Provider(&call))?;
                let object = self.openai.create_file(&client, &data).await?;
                Ok(WriteReply::Object(object))
            }
            WriteCall::Upload(UploadTarget::Azure { call, data, params }) => {
                let opened = self.open_provider(
                    &call,
                    litellm_params(
                        params.credentials.as_ref(),
                        params.max_retries,
                        call.timeout,
                    ),
                )?;
                let reply =
                    azure_call(&opened, &self.client, AzureOperation::Create(&data)).await?;
                azure_write(reply)
            }
            WriteCall::Upload(UploadTarget::Config(upload)) => Err(DispatchError::Unbuilt {
                config: upload.config.class_name(),
            }),
            WriteCall::Delete(DeleteTarget::OpenAi { call, file_id }) => {
                let client = openai_client(ClientSource::Provider(&call))?;
                let deleted = self.openai.delete_file(&client, file_id).await?;
                Ok(WriteReply::Deleted(deleted))
            }
            WriteCall::Delete(DeleteTarget::Azure {
                call,
                file_id,
                params,
                ..
            }) => {
                let client_params =
                    litellm_params(params.credentials, call.max_retries, call.timeout);
                let opened = self.open_provider(&call, client_params)?;
                let reply =
                    azure_call(&opened, &self.client, AzureOperation::Delete(file_id)).await?;
                azure_write(reply)
            }
            WriteCall::Delete(DeleteTarget::Config(delete)) => Err(DispatchError::Unbuilt {
                config: delete.config.class_name(),
            }),
        }
    }
}

// the azure handler answers create_file and delete_file with an object or a deleted marker
fn azure_write(reply: AzureReply) -> Result<WriteReply, DispatchError> {
    match reply {
        AzureReply::Object(object) => Ok(WriteReply::Object(object)),
        AzureReply::Deleted(deleted) => Ok(WriteReply::Deleted(deleted)),
        AzureReply::Content(_) | AzureReply::Page(_) => Err(DispatchError::Reply),
    }
}

impl FileWriteProviders for FileCallDispatch {
    type Config = FileProviderConfig;
    type Logger = ();
    type Client = ();
    type Error = DispatchError;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config> {
        file_config_selection("", provider)
    }

    async fn upload(&self, target: UploadTarget<'_, Self>) -> Result<FileObject, DispatchError> {
        match self.send(WriteCall::Upload(target)).await? {
            WriteReply::Object(object) => Ok(object),
            WriteReply::Deleted(_) => Err(DispatchError::Reply),
        }
    }

    async fn delete(&self, target: DeleteTarget<'_, Self>) -> Result<FileDeleted, DispatchError> {
        match self.send(WriteCall::Delete(target)).await? {
            WriteReply::Deleted(deleted) => Ok(deleted),
            WriteReply::Object(_) => Err(DispatchError::Reply),
        }
    }
}

impl FileReadDispatch for FileCallDispatch {
    type Config = FileProviderConfig;
    type Client = ();
    type Error = DispatchError;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config> {
        file_config_selection("", provider)
    }

    async fn read(
        &self,
        target: ReadTarget<'_, FileProviderConfig, ()>,
        operation: ReadOperation<'_>,
    ) -> Result<ReadAnswer, DispatchError> {
        match target {
            ReadTarget::Compatible(read) => compatible_answer(&self.openai, &read, operation).await,
            ReadTarget::Azure(read) => azure_answer(self, &read, operation).await,
            ReadTarget::Generic(read) => Err(DispatchError::Unbuilt {
                config: read.config.class_name(),
            }),
        }
    }
}

// the OpenAI files client both operations share, whose page or object comes back as the SDK parsed it
async fn compatible_answer(
    openai: &FilesApi<ProxiedClient>,
    read: &CompatibleRead,
    operation: ReadOperation<'_>,
) -> Result<ReadAnswer, DispatchError> {
    let client = openai_client(ClientSource::Compatible(read))?;
    match operation {
        ReadOperation::Retrieve { file_id } => Ok(ReadAnswer::File(Box::new(
            openai.retrieve_file(&client, file_id).await?,
        ))),
        ReadOperation::List { purpose } => {
            let query = FileQuery {
                purpose: purpose.map(str::to_owned),
            };
            Ok(ReadAnswer::Listing(FileListing::Page(Box::new(
                openai.list_files(&client, &query).await?,
            ))))
        }
    }
}
