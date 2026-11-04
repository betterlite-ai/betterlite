use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use thiserror::Error;

pub mod initialization;

pub static GOOGLE_KMS_CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);
pub static GOOGLE_KMS_RESOURCE_NAME: Mutex<Option<String>> = Mutex::new(None);

pub struct Client {
    pub resource_name: String,
    pub transport: Arc<dyn DecryptTransport>,
}

pub type DecryptFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, GoogleKmsError>> + Send + 'a>>;

pub trait DecryptTransport: Send + Sync {
    fn decrypt<'a>(&'a self, resource_name: &'a str, ciphertext: &'a [u8]) -> DecryptFuture<'a>;
}

#[derive(Debug, Error)]
pub enum GoogleKmsError {
    #[error("Missing required environment variable - GOOGLE_APPLICATION_CREDENTIALS")]
    MissingCredentials,
    #[error("Missing required environment variable - GOOGLE_KMS_RESOURCE_NAME")]
    MissingResourceName,
    #[error("Google KMS client initialization failed: {0}")]
    Client(String),
    #[error("Google KMS transport failed")]
    Transport(#[source] std::io::Error),
    #[error("Google KMS client state is poisoned")]
    ClientState,
}
