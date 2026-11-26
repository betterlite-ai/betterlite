use std::collections::BTreeMap;

use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{
    Operation as AzureOperation, Reply as AzureReply, call as azure_call,
};
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_core::api::file_provider_read::target::{
    CompatibleRead, FileListing, FileReadDispatch, ReadAnswer, ReadOperation, ReadTarget,
};
use betterlite_core::api::file_provider_write::{DeleteTarget, FileWriteProviders, UploadTarget};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::FilesApi;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{FileDeleted, FileObject, FileQuery};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection,
};

use self::azure::{azure_answer, open_azure};
use self::clients::{
    compatible_client, delete_litellm_params, is_asynchronous, openai_client, settings,
    write_litellm_params,
};

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

impl FileWriteProviders for FileCallDispatch {
    type Config = FileProviderConfig;
    type Logger = ();
    type Client = ();
    type Error = DispatchError;

    fn files_config(&self, provider: ProviderId) -> Option<Self::Config> {
        file_config_selection("", provider)
    }

    async fn upload(&self, target: UploadTarget<'_, Self>) -> Result<FileObject, DispatchError> {
        match target {
            UploadTarget::OpenAi { call, data } => {
                let client = openai_client(&call)?;
                Ok(self.openai.create_file(&client, &data).await?)
            }
            UploadTarget::Azure {
                call: provider,
                data,
                params,
            } => {
                let opened = open_azure(
                    self,
                    settings(
                        provider.api_key.clone(),
                        provider.api_base.clone(),
                        provider.api_version.clone(),
                    ),
                    is_asynchronous(provider.mode),
                    write_litellm_params(params),
                )?;
                match azure_call(&opened, &self.client, AzureOperation::Create(&data)).await? {
                    AzureReply::Object(object) => Ok(object),
                    AzureReply::Content(_) | AzureReply::Page(_) | AzureReply::Deleted(_) => {
                        Err(DispatchError::Reply)
                    }
                }
            }
            UploadTarget::Config(upload) => Err(DispatchError::Unbuilt {
                config: upload.config.class_name(),
            }),
        }
    }

    async fn delete(&self, target: DeleteTarget<'_, Self>) -> Result<FileDeleted, DispatchError> {
        match target {
            DeleteTarget::OpenAi { call, file_id } => {
                let client = openai_client(&call)?;
                Ok(self.openai.delete_file(&client, file_id).await?)
            }
            DeleteTarget::Azure {
                call: provider,
                file_id,
                params,
                ..
            } => {
                let opened = open_azure(
                    self,
                    settings(
                        provider.api_key.clone(),
                        provider.api_base.clone(),
                        provider.api_version.clone(),
                    ),
                    is_asynchronous(provider.mode),
                    delete_litellm_params(&params, provider.timeout, provider.max_retries),
                )?;
                match azure_call(&opened, &self.client, AzureOperation::Delete(file_id)).await? {
                    AzureReply::Deleted(deleted) => Ok(deleted),
                    AzureReply::Object(_) | AzureReply::Content(_) | AzureReply::Page(_) => {
                        Err(DispatchError::Reply)
                    }
                }
            }
            DeleteTarget::Config(delete) => Err(DispatchError::Unbuilt {
                config: delete.config.class_name(),
            }),
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
    let client = compatible_client(read)?;
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
