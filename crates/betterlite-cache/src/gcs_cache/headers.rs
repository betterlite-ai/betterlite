use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_client::gcs_storage_auth::error::GcsClientError;
use betterlite_logging::gcs_client::gcs_storage_auth::sync_header_behaviors::sync_construct_request_headers;
use betterlite_logging::gcs_logger::transport::PooledGcsClient;
use betterlite_logging::gcs_mock::auth::VertexAuth;
use betterlite_logging::gcs_mock::client::GcsHttpClient;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;
use tokio::task::JoinError;

// the sync header operation asks for GOOGLE_SECRET_MANAGER_PROJECT_ID on every call
#[derive(Debug)]
pub(super) struct ProcessEnvironment;

impl EnvironmentSource for ProcessEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        process_environment().remove(name)
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        process_environment()
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum HeaderError<Auth>
where
    Auth: VertexAuth,
{
    #[error("the GCS request headers could not be built")]
    Build {
        #[source]
        source: GcsClientError<Auth, PooledGcsClient>,
    },
    #[error("the GCS header lookup task failed")]
    Task {
        #[source]
        source: JoinError,
    },
    #[error("a GCS request header name is not valid")]
    Name {
        #[source]
        source: InvalidHeaderName,
    },
    #[error("a GCS request header value is not valid")]
    Value {
        #[source]
        source: InvalidHeaderValue,
    },
}

pub(super) async fn build_headers<Auth>(
    base: &Arc<GcsBucketBase<Auth, PooledGcsClient>>,
    bucket_name: String,
    service_account_path: Option<SecretString>,
) -> Result<HeaderMap, HeaderError<Auth>>
where
    Auth: VertexAuth + Clone + Send + Sync + 'static,
{
    let base = Arc::clone(base);
    let runtime = Handle::current();
    let request = runtime.spawn_blocking(move || {
        sync_construct_request_headers(&base, &bucket_name, service_account_path.as_ref())
    });
    let headers = request
        .await
        .map_err(|source| HeaderError::Task { source })?
        .map_err(|source| HeaderError::Build { source })?;
    let mut map = HeaderMap::new();
    for (name, value) in &headers {
        map.insert(
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|source| HeaderError::Name { source })?,
            HeaderValue::from_str(value.expose_secret())
                .map_err(|source| HeaderError::Value { source })?,
        );
    }
    Ok(map)
}
