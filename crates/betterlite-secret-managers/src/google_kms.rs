use std::sync::{Arc, Mutex};

use google_cloud_kms_v1::client::KeyManagementService;
use thiserror::Error;

pub mod initialization;

pub static GOOGLE_KMS_CLIENT: Mutex<Option<Arc<KeyManagementService>>> = Mutex::new(None);
pub static GOOGLE_KMS_RESOURCE_NAME: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Error)]
pub enum GoogleKmsError {
    #[error("Missing required environment variable - GOOGLE_APPLICATION_CREDENTIALS")]
    MissingCredentials,
    #[error("Missing required environment variable - GOOGLE_KMS_RESOURCE_NAME")]
    MissingResourceName,
    #[error("Google KMS client initialization failed: {0}")]
    Client(String),
    #[error("Google KMS client state is poisoned")]
    ClientState,
}
