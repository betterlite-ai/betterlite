use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::gcs_client::gcs_storage_auth::GcsBucketBase;
use betterlite_logging::gcs_client::gcs_storage_auth::download_behaviors::download_gcs_object;
use betterlite_logging::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use betterlite_logging::gcs_client::gcs_storage_auth::initialization_behaviors::initialize as build_bucket;
use betterlite_logging::gcs_mock::auth::VertexAuth;
use betterlite_logging::gcs_mock::client::GcsHttpClient;
use betterlite_logging::types::gcs_storage_auth::type_definitions::GcsLoggingKwargs;

use super::object::BucketObjectReader;

pub struct GcsReader<Auth, Http> {
    pub bucket_name: String,
    pub bucket: Option<GcsBucketBase<Auth, Http>>,
}

impl<Auth, Http> GcsReader<Auth, Http>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    #[must_use]
    pub fn initialize(
        environment: Arc<dyn EnvironmentSource>,
        bucket_name: &str,
        auth: Auth,
        http: Http,
    ) -> Self {
        match build_bucket(environment, Some(bucket_name.to_owned()), auth, http) {
            Ok(bucket) => Self {
                bucket_name: bucket_name.to_owned(),
                bucket: Some(bucket),
            },
            Err(error) => {
                tracing::error!(
                    bucket_name,
                    cause = %error,
                    "Error creating the GCS client for bucket"
                );
                Self {
                    bucket_name: bucket_name.to_owned(),
                    bucket: None,
                }
            }
        }
    }
}

#[async_trait]
impl<Auth, Http> BucketObjectReader for GcsReader<Auth, Http>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    async fn read_object(&self, object_key: &str) -> Option<Vec<u8>> {
        let bucket = self.bucket.as_ref()?;
        download_gcs_object(bucket, object_key, &GcsLoggingKwargs::default())
            .await
            .map_or_else(
                || {
                    tracing::error!(
                        bucket_name = %self.bucket_name,
                        object_key,
                        "Error retrieving object from GCS bucket"
                    );
                    None
                },
                Some,
            )
    }
}

pub trait GcsBucketOpener: Send + Sync {
    fn open(&self, bucket_name: &str) -> Box<dyn BucketObjectReader>;
}

pub struct GcsReaderFactory<Auth, Http> {
    pub environment: Arc<dyn EnvironmentSource>,
    pub auth: Auth,
    pub http: Http,
}

impl<Auth, Http> GcsBucketOpener for GcsReaderFactory<Auth, Http>
where
    Auth: VertexAuth + Clone + 'static,
    Http: GcsHttpClient + Clone + 'static,
{
    fn open(&self, bucket_name: &str) -> Box<dyn BucketObjectReader> {
        Box::new(GcsReader::initialize(
            Arc::clone(&self.environment),
            bucket_name,
            self.auth.clone(),
            self.http.clone(),
        ))
    }
}

mod tests;
