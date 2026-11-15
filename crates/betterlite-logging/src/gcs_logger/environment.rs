use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_client::gcs_storage_auth::{BUCKET_NAME_ENV, SERVICE_ACCOUNT_ENV};
use crate::gcs_mock::settings::{ENABLE_ENV, LATENCY_ENV};

// the base reads the bucket, the credentials and the two mock settings out of a snapshot when it initializes
const SNAPSHOT_KEYS: [&str; 4] = [
    BUCKET_NAME_ENV,
    SERVICE_ACCOUNT_ENV,
    ENABLE_ENV,
    LATENCY_ENV,
];

pub struct CatalogEnvironment {
    pub(crate) lookup: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl EnvironmentSource for CatalogEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        self.lookup.lookup(name)
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        SNAPSHOT_KEYS
            .iter()
            .filter_map(|name| Some(((*name).to_owned(), self.get(name)?)))
            .collect()
    }
}

mod tests;
