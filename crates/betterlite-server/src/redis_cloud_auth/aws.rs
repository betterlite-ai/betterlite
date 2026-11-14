use aws_credential_types::Credentials;
use aws_credential_types::provider::{ProvideCredentials as _, SharedCredentialsProvider};
use betterlite_config::sections::redis_credentials::aws::{
    AwsCredentialsResolver, ElastiCacheSigningError,
};
use tokio::runtime::Handle;

pub struct SdkAwsCredentials {
    pub runtime: Handle,
    pub provider: SharedCredentialsProvider,
}

impl AwsCredentialsResolver for SdkAwsCredentials {
    fn resolve(&self) -> Result<Credentials, ElastiCacheSigningError> {
        self.runtime
            .block_on(self.provider.provide_credentials())
            .map_err(|source| ElastiCacheSigningError::Credentials { source })
    }
}

pub async fn load_default_provider() -> Option<SharedCredentialsProvider> {
    aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await
        .credentials_provider()
}
