use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_logger::auth::GoogleCloudAuth;
use betterlite_logging::gcs_logger::transport::PooledGcsClient;
use betterlite_pyhost::instance_ref::Storage;
use betterlite_pyhost::remote_module::{ModuleFetcher, ModuleRequest};
use thiserror::Error;
use tokio::runtime::Builder;

use crate::bucket_config::gcs_reader::{GcsBucketOpener, GcsReader};
use crate::bucket_config::module_download::{BucketModuleDownloader, ModuleDownloadError};
use crate::bucket_config::object::BucketObjectReader;
use crate::bucket_config::s3_reader::ProxiedS3Get;
use crate::bucket_config::source::BucketType;

pub struct BucketModuleFetcher {
    source: FetchSource,
}

enum FetchSource {
    Environment(BTreeMap<String, String>),
    Supplied(Arc<BucketModuleDownloader>),
}

impl BucketModuleFetcher {
    #[must_use]
    pub const fn production(environment: BTreeMap<String, String>) -> Self {
        Self {
            source: FetchSource::Environment(environment),
        }
    }

    #[must_use]
    pub const fn supplied(downloader: Arc<BucketModuleDownloader>) -> Self {
        Self {
            source: FetchSource::Supplied(downloader),
        }
    }

    async fn download(&self, request: &ModuleRequest) -> Result<(), BucketFetchError> {
        let downloader = match &self.source {
            FetchSource::Environment(environment) => Arc::new(downloader_for(environment)?),
            FetchSource::Supplied(downloader) => Arc::clone(downloader),
        };
        downloader
            .download_module(
                bucket_type(request.storage),
                &request.bucket,
                &request.object_key,
                &request.local_path,
            )
            .await
            .map_err(BucketFetchError::Download)
    }
}

impl ModuleFetcher for BucketModuleFetcher {
    type Error = BucketFetchError;

    fn fetch(&self, request: &ModuleRequest) -> Result<(), Self::Error> {
        // the loader calls this from a pool thread, so a private runtime drives the async download
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| BucketFetchError::Runtime { source })?;
        runtime.block_on(self.download(request))
    }
}

// LiteLLM builds a fresh client and fresh credentials for every module download
fn downloader_for(
    environment: &BTreeMap<String, String>,
) -> Result<BucketModuleDownloader, BucketFetchError> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), environment)?);
    let client = ProxiedClient::outbound(&policy)?;
    Ok(BucketModuleDownloader {
        environment: environment.clone(),
        s3: Arc::new(ProxiedS3Get {
            client: client.clone(),
        }),
        gcs: Arc::new(GcsModuleOpener {
            environment: Arc::new(environment.clone()),
            client,
        }),
    })
}

const fn bucket_type(storage: Storage) -> BucketType {
    match storage {
        Storage::S3 => BucketType::S3,
        Storage::Gcs => BucketType::Gcs,
    }
}

struct GcsModuleOpener {
    environment: Arc<dyn EnvironmentSource>,
    client: ProxiedClient,
}

impl GcsBucketOpener for GcsModuleOpener {
    fn open(&self, bucket_name: &str) -> Box<dyn BucketObjectReader> {
        Box::new(GcsReader::initialize(
            Arc::clone(&self.environment),
            bucket_name,
            GoogleCloudAuth::default(),
            PooledGcsClient {
                client: self.client.clone(),
            },
        ))
    }
}

#[derive(Debug, Error)]
pub enum BucketFetchError {
    #[error("the Python module download runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
    #[error("the bucket proxy settings could not be read: {source}")]
    Proxy {
        #[from]
        source: EnvError,
    },
    #[error("the HTTP client for the bucket download could not be built: {source}")]
    Client {
        #[from]
        source: HttpError,
    },
    #[error(transparent)]
    Download(#[from] ModuleDownloadError),
}

mod tests;
