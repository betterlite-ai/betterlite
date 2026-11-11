use std::fmt::{Debug, Formatter, Result as FormatResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::s3_transport::signing::{CredentialError, CredentialSource, Credentials};
use crate::s3_v2::connection::Credentials as ConnectionCredentials;

const ACCESS_KEY_VARIABLE: &str = "AWS_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY_VARIABLE: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN_VARIABLE: &str = "AWS_SESSION_TOKEN";

pub struct ConfiguredKeys {
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl Debug for ConfiguredKeys {
    fn fmt(&self, f: &mut Formatter<'_>) -> FormatResult {
        f.debug_struct("ConfiguredKeys")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .field("session_token", &self.session_token)
            .finish_non_exhaustive()
    }
}

// the profile, AssumeRole and web-identity flows need a credential chain no logging task has
#[must_use]
pub fn unsupported_flow(credentials: &ConnectionCredentials) -> Option<&'static str> {
    [
        (
            "s3_aws_profile_name",
            credentials.aws_profile_name.is_some(),
        ),
        ("s3_aws_role_name", credentials.aws_role_name.is_some()),
        (
            "s3_aws_web_identity_token",
            credentials.aws_web_identity_token.is_some(),
        ),
        (
            "s3_aws_sts_endpoint",
            credentials.aws_sts_endpoint.is_some(),
        ),
    ]
    .into_iter()
    .find_map(|(setting, configured)| configured.then_some(setting))
}

impl CredentialSource for ConfiguredKeys {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>> {
        Box::pin(async move {
            let environment = self.environment.as_ref();
            let configured = (
                self.access_key_id.clone().or_else(|| {
                    environment
                        .lookup(ACCESS_KEY_VARIABLE)
                        .map(SecretString::from)
                }),
                self.secret_access_key.clone().or_else(|| {
                    environment
                        .lookup(SECRET_ACCESS_KEY_VARIABLE)
                        .map(SecretString::from)
                }),
                self.session_token.clone().or_else(|| {
                    environment
                        .lookup(SESSION_TOKEN_VARIABLE)
                        .map(SecretString::from)
                }),
            );
            let (Some(access_key_id), Some(secret_access_key), session_token) = configured else {
                return Err(CredentialError::Unavailable);
            };
            Ok(Credentials {
                access_key_id,
                secret_access_key,
                session_token,
            })
        })
    }
}

mod tests;
