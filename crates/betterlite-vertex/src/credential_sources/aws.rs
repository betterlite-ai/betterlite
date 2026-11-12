use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError as BedrockCredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;

use super::error::CredentialSourceError;
use super::sdk::{CredentialClock, TokenSource};
use crate::aws_federation::AwsCredentialsResolver;
use crate::aws_federation::exchange;
use crate::aws_federation::{FederationInput, credentials_from_explicit_aws};

pub trait AwsCredentialsSource: Send + Sync {
    fn credentials<'deps>(
        &'deps self,
        settings: &'deps AwsSettings,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, BedrockCredentialError>> + Send + 'deps>>;
}

impl<R> AwsCredentialsSource for R
where
    R: AwsCredentialsResolver + Send + Sync + 'static,
{
    fn credentials<'deps>(
        &'deps self,
        settings: &'deps AwsSettings,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, BedrockCredentialError>> + Send + 'deps>>
    {
        Box::pin(AwsCredentialsResolver::credentials(self, settings))
    }
}

pub(crate) struct AwsWifTokenSource {
    pub(crate) input: FederationInput,
    pub(crate) resolver: Arc<dyn AwsCredentialsSource>,
    pub(crate) client: ProxiedClient,
    pub(crate) clock: Arc<dyn CredentialClock>,
}

impl TokenSource for AwsWifTokenSource {
    fn access_token(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<SecretString, CredentialSourceError>> + Send + '_>>
    {
        Box::pin(async move {
            // every refresh resolves the AWS credentials again, so an STS rotation reaches the exchange
            let resolver = SharedResolver(Arc::clone(&self.resolver));
            let federation = credentials_from_explicit_aws(self.input.clone(), &resolver)
                .map_err(CredentialSourceError::Federation)?;
            let token = exchange::access_token(&federation, &self.client, self.clock.now())
                .await
                .map_err(CredentialSourceError::Federation)?;
            Ok(token.token)
        })
    }
}

struct SharedResolver(Arc<dyn AwsCredentialsSource>);

impl AwsCredentialsResolver for SharedResolver {
    async fn credentials(
        &self,
        settings: &AwsSettings,
    ) -> Result<Credentials, BedrockCredentialError> {
        self.0.credentials(settings).await
    }
}
