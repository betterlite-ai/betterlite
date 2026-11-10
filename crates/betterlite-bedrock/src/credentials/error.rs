use aws_sdk_sts::error::{BuildError, SdkError};
use aws_sdk_sts::operation::assume_role::AssumeRoleError;

use crate::assume_role::error::ExpiryError;
use crate::aws_settings::AuthError;
use crate::aws_settings::credential_cache::CacheError;
use crate::aws_settings::sources::SourceError;
use crate::web_identity::error::WebIdentityError;

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    WebIdentity(#[from] WebIdentityError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Expiry(#[from] ExpiryError),
    #[error("the STS assume role call failed")]
    AssumeRole {
        #[source]
        source: Box<SdkError<AssumeRoleError>>,
    },
    #[error("the STS assume role response has no credentials")]
    MissingCredentials,
    #[error("the STS session tag does not build")]
    Tag {
        #[source]
        source: BuildError,
    },
}
