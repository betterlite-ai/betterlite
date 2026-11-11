use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::s3_transport::signing::{CredentialError, CredentialSource, Credentials};

mod tests;

pub const ACCESS_KEY_VARIABLE: &str = "AWS_ACCESS_KEY_ID";
pub const SECRET_ACCESS_KEY_VARIABLE: &str = "AWS_SECRET_ACCESS_KEY";
pub const SESSION_TOKEN_VARIABLE: &str = "AWS_SESSION_TOKEN";

pub struct ConfiguredCredentials {
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
}

// an environment lookup has no Debug, and the secret fields stay redacted through SecretString
impl fmt::Debug for ConfiguredCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguredCredentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .field("session_token", &self.session_token)
            .finish_non_exhaustive()
    }
}

impl ConfiguredCredentials {
    fn identity(&self) -> Result<Credentials, CredentialError> {
        match (&self.access_key_id, &self.secret_access_key) {
            (Some(access), Some(secret)) => Ok(Credentials {
                access_key_id: access.clone(),
                secret_access_key: secret.clone(),
                session_token: self.session_token.clone(),
            }),
            (None, None) => {
                let access = self.environment.lookup(ACCESS_KEY_VARIABLE);
                let secret = self.environment.lookup(SECRET_ACCESS_KEY_VARIABLE);
                match (access, secret) {
                    (Some(access_key), Some(secret_key)) => Ok(Credentials {
                        access_key_id: SecretString::from(access_key),
                        secret_access_key: SecretString::from(secret_key),
                        session_token: self
                            .environment
                            .lookup(SESSION_TOKEN_VARIABLE)
                            .map(SecretString::from),
                    }),
                    _ => Err(CredentialError::Unavailable),
                }
            }
            _ => Err(CredentialError::Unavailable),
        }
    }
}

impl CredentialSource for ConfiguredCredentials {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>> {
        Box::pin(async move { self.identity() })
    }
}
