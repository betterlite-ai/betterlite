use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use secrecy::SecretString;

use crate::gcs_mock::auth::{GcsMockAuth, VertexAuth};
use crate::gcs_mock::client::{GcsHttpClient, create_mock_gcs_client};
use crate::gcs_mock::settings::GcsMockSettings;

use super::environment_source::EnvironmentSource;
use super::error::GcsClientError;
use super::{BUCKET_NAME_ENV, GcsBucketBase, SERVICE_ACCOUNT_ENV};

pub fn initialize<Auth, Http>(
    environment: Arc<dyn EnvironmentSource>,
    bucket_name: Option<String>,
    auth: Auth,
    http: Http,
) -> Result<GcsBucketBase<Auth, Http>, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let snapshot = environment.snapshot();
    let settings = GcsMockSettings::from_environment(&snapshot).map_err(GcsClientError::Mock)?;
    let mut mock_auth = GcsMockAuth {
        real: auth,
        installed: false,
    };
    if settings.enabled {
        let installation = mock_auth.install();
        tracing::debug!(?installation, "installed the GCS mock authentication");
    }
    let client = create_mock_gcs_client(&snapshot, Some(http)).map_err(GcsClientError::Mock)?;
    let resolved_bucket = bucket_name.or_else(|| snapshot.get(BUCKET_NAME_ENV).cloned());
    let service_account = snapshot
        .get(SERVICE_ACCOUNT_ENV)
        .cloned()
        .map(SecretString::from);
    Ok(GcsBucketBase {
        is_mock_mode: settings.enabled,
        bucket_name: resolved_bucket,
        path_service_account: service_account,
        auth: Arc::new(mock_auth),
        client,
        vertex_instances: Mutex::new(BTreeMap::new()),
        environment,
    })
}

mod tests;
