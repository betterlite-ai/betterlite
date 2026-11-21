use betterlite_prometheus::collector_registry::CollectorRegistryError;
use betterlite_prometheus::collector_registry::metric::BucketError;

#[derive(Debug, thiserror::Error)]
pub enum PrometheusServicesError {
    #[error(transparent)]
    Registry(#[from] CollectorRegistryError),
    #[error(transparent)]
    Buckets(#[from] BucketError),
}
