use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use betterlite_sigv4::credentials::Credentials as SigV4Credentials;

mod tests;

pub const S3_SERVICE: &str = "s3";

// clippy::pub_use is denied, so an alias keeps the signing credentials nameable here
pub type Credentials = SigV4Credentials;

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("AWS credentials for S3 could not be resolved")]
    Unavailable,
}

pub trait CredentialSource: Send + Sync + Debug {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>>;
}

#[derive(Clone, Debug)]
pub struct StaticCredentials {
    pub credentials: Credentials,
}

impl CredentialSource for StaticCredentials {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>> {
        let credentials = self.credentials.clone();
        Box::pin(async move { Ok(credentials) })
    }
}

pub trait Clock: Send + Sync + Debug {
    fn now(&self) -> SystemTime;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
