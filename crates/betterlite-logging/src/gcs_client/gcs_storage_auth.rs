use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use secrecy::SecretString;

use crate::gcs_mock::auth::GcsMockAuth;
use crate::gcs_mock::client::GcsMockClient;

use self::caching_key_behaviors::CredentialKey;
use self::environment_source::EnvironmentSource;

pub mod bucket_folder_behaviors;
pub mod caching_key_behaviors;
pub mod config_resolution_behaviors;
pub mod download_behaviors;
pub mod environment_source;
pub mod error;
pub mod header_construction_behaviors;
pub mod initialization_behaviors;
pub mod upload_behaviors;
pub mod vertex_caching_behaviors;

mod tests;

pub const IAM_AUTH_KEY: &str = "IAM_AUTH";
pub const BUCKET_NAME_ENV: &str = "GCS_BUCKET_NAME";
pub const SERVICE_ACCOUNT_ENV: &str = "GCS_PATH_SERVICE_ACCOUNT";
pub const PROJECT_ID_ENV: &str = "GOOGLE_SECRET_MANAGER_PROJECT_ID";
pub const TOKEN_MODEL: &str = "gcs-bucket";
pub const STORAGE_ENDPOINT: &str = "https://storage.googleapis.com";
pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";

pub type RequestHeaders = BTreeMap<String, SecretString>;
pub type VertexInstance<Auth> = Arc<GcsMockAuth<Auth>>;

pub struct GcsBucketBase<Auth, Http> {
    pub is_mock_mode: bool,
    pub bucket_name: Option<String>,
    pub path_service_account: Option<SecretString>,
    pub auth: VertexInstance<Auth>,
    pub client: GcsMockClient<Http>,
    pub vertex_instances: Mutex<BTreeMap<CredentialKey, VertexInstance<Auth>>>,
    pub environment: Arc<dyn EnvironmentSource>,
}
