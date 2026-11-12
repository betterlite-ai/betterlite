use std::sync::Arc;

use secrecy::SecretString;

use crate::azure_client_secret::SecretResolver;
use crate::azure_default_chain::environment::ChainEnvironment;
use crate::azure_default_chain::error::DefaultChainError;
use crate::azure_default_chain::grant::{
    ClientSecretRecord, Grant, GrantError, Grants, LinkError, UsernamePasswordRecord,
};
use crate::azure_default_chain::provider::{
    AzureTokenProvider, ProviderRequest, get_azure_ad_token_provider, try_default_credential,
};
use crate::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};

use super::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};

pub struct GrantSources<'data, F: Grants> {
    pub environment: &'data ChainEnvironment,
    pub grants: &'data F,
    pub exchange: &'data AzureOidcExchange,
    pub resolver: &'data dyn SecretResolver,
}

struct GrantToken<G> {
    grant: G,
}

struct SdkToken<'data, F: Grants> {
    provider: AzureTokenProvider<'data, F>,
}

impl<G: Grant + Send + Sync> CallableTokenProvider for GrantToken<G> {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        self.grant
            .token()
            .map(|token| TokenReply::Text(token.token))
            .map_err(link_failure)
    }
}

impl<'data, F> CallableTokenProvider for SdkToken<'data, F>
where
    F: Grants + 'data,
    F::Grant<'data>: Send + Sync,
{
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        self.provider
            .token()
            .map(|token| TokenReply::Text(token.token))
            .map_err(|error| sdk_failure(&error))
    }
}

fn link_failure(error: LinkError) -> ProviderFailure {
    match error {
        LinkError::Unavailable(message) | LinkError::Authentication(message) => ProviderFailure {
            class: "ClientAuthenticationError".to_owned(),
            message,
        },
    }
}

fn sdk_failure(error: &DefaultChainError) -> ProviderFailure {
    ProviderFailure {
        class: "ClientAuthenticationError".to_owned(),
        message: error.to_string(),
    }
}

impl<'data, F> CredentialSources for GrantSources<'data, F>
where
    F: Grants + 'data,
    for<'grant> F::Grant<'grant>: Send + Sync,
{
    fn client_secret<'source>(
        &'source self,
        record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        grant_provider(self.grants.client_secret(record))
    }

    fn username_password<'source>(
        &'source self,
        record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        grant_provider(self.grants.username_password(record))
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        let request = ProviderRequest {
            scope: Some(scope),
            credential: None,
            environment: self.environment,
            grants: self.grants,
        };
        match get_azure_ad_token_provider(&request) {
            Ok(provider) => Ok(Arc::new(SdkToken { provider })),
            Err(error) => Err(refresh_failure(&error)),
        }
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        try_default_credential(scope, self.environment, self.grants)
            .provider
            .map(|provider| -> Arc<dyn CallableTokenProvider + 'source> {
                Arc::new(SdkToken { provider })
            })
    }

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.exchange
            .token(input, self.resolver)
            .map_err(|error| SourceFailure {
                class: "AzureOpenAIError".to_owned(),
                status: error.status_code(),
                message: error.to_string(),
            })
    }
}

fn grant_provider<'source, G>(
    grant: Result<G, GrantError>,
) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>
where
    G: Grant + Send + Sync + 'source,
{
    grant
        .map(|resolved| -> Arc<dyn CallableTokenProvider + 'source> {
            Arc::new(GrantToken { grant: resolved })
        })
        .map_err(|error| SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: error.message,
        })
}

fn refresh_failure(error: &DefaultChainError) -> SourceFailure {
    SourceFailure {
        class: refresh_error_class(error).to_owned(),
        status: None,
        message: error.to_string(),
    }
}

// B677 keeps no SDK exception class, so construction counts as ValueError and the rest keep the SDK class
const fn refresh_error_class(error: &DefaultChainError) -> &'static str {
    match error {
        DefaultChainError::InvalidFilter { .. } | DefaultChainError::Construction { .. } => {
            "ValueError"
        }
        DefaultChainError::MissingEnvironment { .. } => "KeyError",
        DefaultChainError::MissingArguments { .. } => "TypeError",
        DefaultChainError::UnsupportedFilter { .. }
        | DefaultChainError::Credential { .. }
        | DefaultChainError::Exhausted { .. } => "RuntimeError",
    }
}
