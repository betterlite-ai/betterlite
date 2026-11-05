use std::{
    future::Future,
    io,
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

pub type DecryptFuture<'request> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, GoogleKmsError>> + Send + 'request>>;

pub trait DecryptTransport: Send + Sync {
    fn decrypt<'request>(
        &'request self,
        resource_name: &'request str,
        ciphertext: &'request [u8],
    ) -> DecryptFuture<'request>;
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
    Transport(#[source] io::Error),
    #[error("Google KMS client state is poisoned")]
    ClientState,
}
