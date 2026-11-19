use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_azure_storage::AzureFileStorageDeps;
use betterlite_core::api::file_storage_service::error::StorageServiceError;
use betterlite_core::api::file_storage_service::factory::StorageBackendDeps;
use betterlite_core::api::file_storage_service::projection::{
    FileMetadata, StoredFile, project_stored_file,
};
use betterlite_core::api::file_storage_service::service::{
    StorageUpload, StorageUploadDeps, upload_file_to_storage_backend,
};
use betterlite_db::auxiliary_repository::file_content::ManagedFileContentRepository;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::azure_storage_logging::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider,
};
use betterlite_logging::azure_storage_logging::transport::PooledTransport;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_openai::files::records::FilesPurpose;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    content: String,
    filename: String,
    content_type: String,
    purpose: FilesPurpose,
    target_storage: String,
    storage_url: String,
    target_model_names: Vec<String>,
    environment: BTreeMap<String, String>,
    now: f64,
    uuid: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Projection,
    Upload,
}

struct FixedClock {
    now: f64,
}

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        Duration::try_from_secs_f64(self.now).unwrap_or_default()
    }
}

struct FixedIds {
    uuid: String,
}

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        self.uuid.clone()
    }
}

// the refusal is raised before any upload, so neither a token nor a request is ever taken
struct UnusedChain;

impl ChainProviderFactory for UnusedChain {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        Err(ChainError::Unavailable)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.operation {
            Operation::Projection => Ok(stored_answer(&project(input))),
            Operation::Upload => upload_answer(input),
        }
    }
}

fn project(input: &Input) -> StoredFile {
    let clock = FixedClock { now: input.now };
    let ids = FixedIds {
        uuid: input.uuid.clone(),
    };
    project_stored_file(
        FileMetadata {
            content: input.content.as_bytes(),
            filename: &input.filename,
            purpose: input.purpose,
            target_storage: &input.target_storage,
            storage_url: &input.storage_url,
        },
        &clock,
        &ids,
    )
}

fn stored_answer(stored: &StoredFile) -> Value {
    json!({
        "file": &stored.file,
        "storage_backend": &stored.storage_backend,
        "storage_url": &stored.storage_url,
    })
}

fn refusal_answer(error: &ProxyException) -> Value {
    json!({
        "error": {
            "message": error.message,
            "type": error.error_type,
            "param": error.param,
            "code": error.code,
        }
    })
}

fn upload_answer(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    runtime.block_on(upload(input))
}

async fn upload(input: &Input) -> HelperResult {
    let backend = configured_backend(input)?;
    let clock = FixedClock { now: input.now };
    let ids = FixedIds {
        uuid: input.uuid.clone(),
    };
    let deps = StorageUploadDeps {
        backend,
        clock: &clock,
        ids: &ids,
    };
    let request = StorageUpload {
        content: input.content.as_bytes(),
        filename: Some(&input.filename),
        content_type: Some(&input.content_type),
        purpose: input.purpose,
        target_storage: &input.target_storage,
        target_model_names: &input.target_model_names,
    };
    match upload_file_to_storage_backend(request, deps).await {
        Ok(stored) => Ok(stored_answer(&stored)),
        Err(StorageServiceError::Proxy(error)) => Ok(refusal_answer(&error)),
        Err(StorageServiceError::Upload(error)) => Err::<Value, _>(error).or_value_error(),
    }
}

fn configured_backend(
    input: &Input,
) -> Result<StorageBackendDeps<ManagedFileContentRepository<'static>>, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &input.environment).or_config_error()?,
    );
    let client = ProxiedClient::outbound(&policy).or_config_error()?;
    Ok(StorageBackendDeps {
        azure: AzureFileStorageDeps {
            transport: Arc::new(PooledTransport { client }),
            clock: Arc::new(FixedClock { now: input.now }),
            environment: Arc::new(input.environment.clone()),
            ids: Arc::new(FixedIds {
                uuid: input.uuid.clone(),
            }),
            chain_factory: Arc::new(UnusedChain),
        },
        azure_settings: None,
        database: None,
    })
}
