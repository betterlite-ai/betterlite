use std::io::Error as IoError;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncReadExt as _;

pub const MAX_CA_BYTES: u64 = 0x0010_0000;

#[rustfmt::skip]
#[derive(Debug, Clone)]
pub enum TokenTrust { System, Insecure, Certificate(Arc<[u8]>) }
#[rustfmt::skip]
pub struct TokenExchangeRequest { pub url: String, pub body: String, pub trust: TokenTrust, pub timeout: Duration }
#[rustfmt::skip]
pub struct TokenExchangeResponse { pub status: u16, pub body: Vec<u8> }
#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum TransportFailure {
    #[error("the GDC Gemini token request could not be sent")] Request { #[source] source: Box<TransportError> },
}

#[async_trait]
pub trait TokenTransport: Send + Sync {
    async fn exchange(
        &self,
        request: TokenExchangeRequest,
    ) -> Result<TokenExchangeResponse, TransportFailure>;
}

#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum CaError {
    #[error("the CA certificate {path} could not be read")] Read { path: String, #[source] source: IoError },
    #[error("the CA certificate {path} is larger than {limit} bytes")] TooLarge { path: String, limit: u64 },
}

#[async_trait]
pub trait CaCertificates: Send + Sync {
    async fn load(&self, path: &str) -> Result<Vec<u8>, CaError>;
}

// A local CA bundle is read through the task's bounded reader, never by the caller of a token exchange
pub struct FileCaCertificates;

#[rustfmt::skip]
#[async_trait]
impl CaCertificates for FileCaCertificates {
    async fn load(&self, path: &str) -> Result<Vec<u8>, CaError> {
        let file = File::open(path).await.map_err(|source| CaError::Read { path: path.to_owned(), source })?;
        let mut contents = Vec::new();
        file.take(MAX_CA_BYTES.saturating_add(1)).read_to_end(&mut contents).await.map_err(|source| CaError::Read { path: path.to_owned(), source })?;
        let too_large = u64::try_from(contents.len()).unwrap_or(u64::MAX) > MAX_CA_BYTES;
        if too_large {
            return Err(CaError::TooLarge { path: path.to_owned(), limit: MAX_CA_BYTES });
        }
        Ok(contents)
    }
}

pub trait GdcClock: Send + Sync {
    fn unix_seconds(&self) -> i64;
    fn monotonic_millis(&self) -> u64;
}

#[async_trait]
pub trait GdcSleeper: Send + Sync {
    async fn sleep(&self, delay: Duration);
}

pub trait GdcJitter: Send + Sync {
    fn uniform_millis(&self, low_inclusive: u64, high_inclusive: u64) -> u64;
}
