use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::api::file_azure_storage::Settings;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_azure_storage::AzureFileStorageDeps;
use betterlite_core::api::file_azure_storage::error::AzureFileStorageError;
use betterlite_core::api::file_storage_service::error::{StorageServiceError, StorageUploadError};
use betterlite_core::api::file_storage_service::factory::StorageBackendDeps;
use betterlite_core::api::file_storage_service::projection::StoredFile;
use betterlite_core::api::file_storage_service::service::{
    StorageUpload, StorageUploadDeps, upload_file_to_storage_backend,
};
use betterlite_db::auxiliary_repository::file_content::ManagedFileContentRepository;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::azure_storage_logging::filename::{IdSource, RandomIds};
use betterlite_logging::azure_storage_logging::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider,
};
use betterlite_logging::azure_storage_logging::transport::PooledTransport;
use betterlite_logging::datadog_logging::clock::SystemClock;
use betterlite_openai::files::records::FilesPurpose;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::result::{FileStorageFailure, FileStorageRun};

pub type Outcome = FileStorageRun;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Call {
    Factory(Box<FactoryCall>),
    Upload(Box<UploadCall>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactoryCall {
    pub(crate) backend_type: String,
    pub(crate) target_model_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadCall {
    pub(crate) file_content: Vec<u8>,
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) target_storage: String,
    pub(crate) target_model_names: Vec<String>,
    pub(crate) uuid: String,
}

struct FixedIds(String);

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        self.0.clone()
    }
}

struct StorageToken;

impl ChainTokenProvider for StorageToken {
    fn token(&self) -> Result<SecretString, ChainError> {
        Ok(SecretString::from("tok-1"))
    }
}

struct StorageChain;

impl ChainProviderFactory for StorageChain {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        Ok(Arc::new(StorageToken))
    }
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        send(self, api_base, env)
    }
}

// both sides resolve the factory against the case env, and the upload writes through the service
async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let prepared = prepare(call, api_base, env)?;
    let clock = SystemClock;
    let projection_ids = RandomIds;
    let deps = StorageUploadDeps {
        backend: backend_deps(env, prepared.settings, prepared.ids)?,
        clock: &clock,
        ids: &projection_ids,
    };
    Ok(outcome(
        upload_file_to_storage_backend(prepared.upload, deps).await,
        api_base,
    ))
}

struct Prepared<'call> {
    upload: StorageUpload<'call>,
    settings: Option<Settings>,
    ids: Arc<dyn IdSource>,
}

fn prepare<'call>(
    call: &'call Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Prepared<'call>, SdkHttpError> {
    match call {
        Call::Factory(factory) => Ok(Prepared {
            upload: factory_request(factory),
            settings: None,
            ids: Arc::new(RandomIds),
        }),
        Call::Upload(upload) => {
            let (request, settings) = upload_request(upload, api_base, env)?;
            Ok(Prepared {
                upload: request,
                settings: Some(settings),
                ids: Arc::new(FixedIds(upload.uuid.clone())),
            })
        }
    }
}

fn factory_request(factory: &FactoryCall) -> StorageUpload<'_> {
    storage_upload(
        &[],
        None,
        None,
        &factory.backend_type,
        &factory.target_model_names,
    )
}

fn upload_request<'call>(
    upload: &'call UploadCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<(StorageUpload<'call>, Settings), SdkHttpError> {
    let mut settings = Settings::from_environment(env)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    // the mock origin stands in for the account host, and both service prefixes stay distinct
    settings.dfs_endpoint = format!("{api_base}/.dfs.");
    settings.blob_endpoint = format!("{api_base}/.blob.");
    Ok((
        storage_upload(
            &upload.file_content,
            Some(&upload.filename),
            Some(&upload.content_type),
            &upload.target_storage,
            &upload.target_model_names,
        ),
        settings,
    ))
}

const fn storage_upload<'call>(
    content: &'call [u8],
    filename: Option<&'call str>,
    content_type: Option<&'call str>,
    target_storage: &'call str,
    target_model_names: &'call [String],
) -> StorageUpload<'call> {
    StorageUpload {
        content,
        filename,
        content_type,
        purpose: FilesPurpose::UserData,
        target_storage,
        target_model_names,
    }
}

fn backend_deps(
    env: &BTreeMap<String, String>,
    settings: Option<Settings>,
    ids: Arc<dyn IdSource>,
) -> Result<StorageBackendDeps<ManagedFileContentRepository<'static>>, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    Ok(StorageBackendDeps {
        azure: AzureFileStorageDeps {
            transport: Arc::new(PooledTransport { client }),
            clock: Arc::new(SystemClock),
            environment: Arc::new(env.clone()),
            ids,
            chain_factory: Arc::new(StorageChain),
        },
        azure_settings: settings,
        database: None,
    })
}

fn outcome(result: Result<StoredFile, StorageServiceError>, api_base: &str) -> SdkResult {
    let value = match result {
        Ok(stored) => FileStorageRun {
            url: Some(
                stored
                    .storage_url
                    .strip_prefix(api_base)
                    .map_or_else(|| stored.storage_url.clone(), str::to_owned),
            ),
            downloaded: None,
            error: None,
        },
        Err(StorageServiceError::Proxy(proxy)) => FileStorageRun {
            url: None,
            downloaded: None,
            error: Some(FileStorageFailure {
                message: proxy.message.clone(),
                status: None,
            }),
        },
        Err(StorageServiceError::Upload(error)) => FileStorageRun {
            url: None,
            downloaded: None,
            error: Some(FileStorageFailure {
                message: error.to_string(),
                status: match error.as_ref() {
                    StorageUploadError::Azure(AzureFileStorageError::Status { status, .. }) => {
                        Some(*status)
                    }
                    StorageUploadError::Azure(_) | StorageUploadError::Database(_) => None,
                },
            }),
        },
    };
    SdkResult::FileStorageService {
        value: Box::new(value),
    }
}
