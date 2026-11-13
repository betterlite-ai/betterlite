use std::error::Error;
use std::future::Future;

use secrecy::SecretString;

use super::error::CyberArkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCertificate {
    pub certificate_path: String,
    pub key_path: String,
}

#[derive(Debug)]
pub struct ConjurRequest<'borrowed> {
    pub method: HttpMethod,
    pub url: String,
    pub authorization: Option<&'borrowed SecretString>,
    pub content_type: Option<&'static str>,
    pub body: Option<&'borrowed SecretString>,
    pub verify_tls: bool,
    pub client_certificate: Option<&'borrowed ClientCertificate>,
}

#[derive(Debug)]
pub struct ConjurResponse {
    pub status: u16,
    pub body: SecretString,
}

impl ConjurResponse {
    pub fn require_success<E>(self) -> Result<Self, CyberArkError<E>> {
        if (200..300).contains(&self.status) {
            Ok(self)
        } else {
            Err(CyberArkError::Status {
                status: self.status,
            })
        }
    }
}

pub trait ConjurTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn send<'borrowed>(
        &'borrowed self,
        request: ConjurRequest<'borrowed>,
    ) -> impl Future<Output = Result<ConjurResponse, Self::Error>> + Send;

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, Self::Error>;
}
