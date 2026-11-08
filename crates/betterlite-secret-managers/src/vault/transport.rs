use std::error::Error;
use std::future::Future;
use std::time::Duration;

use secrecy::SecretString;

use crate::cyberark::transport::ClientCertificate;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultMethod {
    Get,
    Post,
    Delete,
}

#[derive(Debug)]
pub struct VaultRequest {
    pub method: VaultMethod,
    pub url: String,
    pub token: Option<SecretString>,
    pub namespace: Option<String>,
    pub body: Option<SecretString>,
    pub client_certificate: Option<ClientCertificate>,
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub max_response_bytes: usize,
}

#[derive(Debug)]
pub struct VaultResponse {
    pub status: u16,
    pub body: SecretString,
}

pub trait VaultTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn send(
        &self,
        request: VaultRequest,
    ) -> impl Future<Output = Result<VaultResponse, Self::Error>> + Send;

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, Self::Error>;
}
