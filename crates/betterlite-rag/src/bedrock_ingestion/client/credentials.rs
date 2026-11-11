use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;

// B594 resolves and caches the credentials; this seam lets a test drive every key set without an account
#[async_trait]
pub trait RagCredentialResolver: Send + Sync {
    async fn credentials(
        &self,
        settings: &AwsSettings,
        region: &str,
    ) -> Result<Credentials, CredentialError>;
}

#[async_trait]
impl RagCredentialResolver for AwsCredentialResolver<'_> {
    async fn credentials(
        &self,
        settings: &AwsSettings,
        region: &str,
    ) -> Result<Credentials, CredentialError> {
        self.resolve_credentials(settings, Some(region)).await
    }
}
