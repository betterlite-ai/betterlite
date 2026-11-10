pub mod cache;
pub mod error;
pub mod role;
mod sources;
mod tests;

use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::VerifySetting;
use betterlite_secret_managers::secret_lookup::SecretLookup;

use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::credential_cache::CredentialCache;
use crate::aws_settings::s3_static_key_pair;
use crate::aws_settings::sources::{Credentials, NativeSources};

use error::CredentialError;

pub struct AwsCredentialResolver<'deps> {
    pub cache: &'deps CredentialCache,
    pub native: &'deps NativeSources,
    pub secrets: &'deps SecretLookup,
    pub sts: &'deps StsClientKwargs,
    pub max_policy_size: i64,
}

impl AwsCredentialResolver<'_> {
    pub async fn get_credentials(
        &self,
        settings: &AwsSettings,
        ssl_verify: Option<&VerifySetting>,
    ) -> Result<Credentials, CredentialError> {
        tracing::debug!("in get credentials");
        sources::resolve(self, settings, settings.region_name.as_deref(), ssl_verify).await
    }

    pub async fn resolve_credentials(
        &self,
        settings: &AwsSettings,
        region: Option<&str>,
    ) -> Result<Credentials, CredentialError> {
        let effective_region = region.or(settings.region_name.as_deref());
        let resolved = sources::resolve(self, settings, effective_region, None).await?;
        Ok(resolved)
    }

    pub async fn resolve_s3_credentials(
        &self,
        s3: &AwsS3Settings,
        settings: &AwsSettings,
        region: Option<&str>,
    ) -> Result<Credentials, CredentialError> {
        if let Some((access_key_id, secret_access_key)) = s3_static_key_pair(s3) {
            return Ok(Credentials {
                access_key_id,
                secret_access_key,
                session_token: None,
            });
        }
        self.resolve_credentials(settings, region).await
    }
}
