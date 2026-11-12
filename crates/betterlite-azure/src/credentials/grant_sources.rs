use std::sync::Arc;

use secrecy::SecretString;

use crate::azure_client_secret::SecretResolver;
use crate::azure_default_chain::environment::ChainEnvironment;
use crate::azure_default_chain::error::DefaultChainError;
use crate::azure_default_chain::grant::{
    ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError, UsernamePasswordRecord,
};
use crate::azure_default_chain::provider::{
    AzureTokenProvider, ProviderRequest, get_azure_ad_token_provider, try_default_credential,
};
use crate::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};

use super::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};

const CLIENT_AUTH_ERROR: &str = "ClientAuthenticationError";

pub struct GrantSources<'data, F: Grants> {
    pub environment: &'data ChainEnvironment,
    pub grants: &'data F,
    pub exchange: &'data AzureOidcExchange,
    pub resolver: &'data dyn SecretResolver,
}

struct Fetch<T> {
    fetch: T,
}

impl<T: Fn() -> Result<ChainToken, ProviderFailure> + Send + Sync> CallableTokenProvider
    for Fetch<T>
{
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        (self.fetch)().map(|token| TokenReply::Text(token.token))
    }
}

fn client_authentication(message: String) -> ProviderFailure {
    ProviderFailure {
        class: CLIENT_AUTH_ERROR.to_owned(),
        message,
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
        get_azure_ad_token_provider(&request)
            .map(sdk_token)
            .map_err(|error| SourceFailure {
                class: refresh_error_class(&error).to_owned(),
                status: None,
                message: error.to_string(),
            })
    }

    fn default_credential<'source>(
        &'source self,
        scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        try_default_credential(scope, self.environment, self.grants)
            .provider
            .map(sdk_token)
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
            Arc::new(Fetch {
                fetch: move || {
                    resolved.token().map_err(
                        |(LinkError::Unavailable(message) | LinkError::Authentication(message))| {
                            client_authentication(message)
                        },
                    )
                },
            })
        })
        .map_err(|error| SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: error.message,
        })
}

fn sdk_token<'source, F>(
    provider: AzureTokenProvider<'source, F>,
) -> Arc<dyn CallableTokenProvider + 'source>
where
    F: Grants + 'source,
    F::Grant<'source>: Send + Sync,
{
    Arc::new(Fetch {
        fetch: move || {
            provider
                .token()
                .map_err(|error| client_authentication(error.to_string()))
        },
    })
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
