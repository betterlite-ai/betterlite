use std::fmt;

use betterlite_config::sections::azure_default_chain::AzureCredentialType;

use super::chain::Chain;
use super::environment::ChainEnvironment;
use super::error::DefaultChainError;
use super::grant::{ChainToken, Grant as _, Grants, LinkError};
use super::{deployment_plan, plan, single_plan};

pub struct ProviderRequest<'env, 'grants, F: Grants> {
    pub scope: Option<&'env str>,
    pub credential: Option<AzureCredentialType>,
    pub environment: &'env ChainEnvironment,
    pub grants: &'grants F,
}

pub enum ProviderInner<G> {
    Single(G),
    Chain(Chain<G>),
}

pub struct AzureTokenProvider<'grants, F: Grants + 'grants> {
    pub credential: AzureCredentialType,
    pub scope: String,
    pub warnings: Vec<String>,
    pub inner: ProviderInner<F::Grant<'grants>>,
}

// the constructed credentials sit behind the inner field and stay out of the Debug output
impl<F: Grants> fmt::Debug for AzureTokenProvider<'_, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureTokenProvider")
            .field("credential", &self.credential)
            .field("scope", &self.scope)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

impl<F: Grants> AzureTokenProvider<'_, F> {
    pub fn token(&self) -> Result<ChainToken, DefaultChainError> {
        match &self.inner {
            ProviderInner::Single(grant) => grant.token().map_err(credential_error),
            ProviderInner::Chain(chain) => {
                chain
                    .token()
                    .map_err(|failure| DefaultChainError::Exhausted {
                        chain: failure.chain,
                        message: failure.message,
                    })
            }
        }
    }
}

fn credential_error(error: LinkError) -> DefaultChainError {
    match error {
        LinkError::Unavailable(message) => DefaultChainError::Credential {
            unavailable: true,
            message,
        },
        LinkError::Authentication(message) => DefaultChainError::Credential {
            unavailable: false,
            message,
        },
    }
}

pub fn get_azure_ad_token_provider<'grants, F>(
    request: &ProviderRequest<'_, 'grants, F>,
) -> Result<AzureTokenProvider<'grants, F>, DefaultChainError>
where
    F: Grants + 'grants,
{
    // an explicit empty scope is forwarded; only an absent one falls back to AZURE_SCOPE then the default
    let scope = request.environment.resolved_scope(request.scope);
    let credential = request
        .credential
        .or(request.environment.credential)
        .unwrap_or_else(|| request.environment.infer_credential_type());
    tracing::info!(
        "For Azure AD Token Provider, choosing credential type: {}",
        credential.as_str()
    );
    let (inner, warnings) = match credential {
        AzureCredentialType::DefaultAzureCredential => {
            let chain = plan::default_chain(request.environment, request.grants, &scope)?;
            let warnings = chain.warnings.clone();
            (ProviderInner::Chain(chain), warnings)
        }
        AzureCredentialType::DeploymentIdentityCredential => {
            let chain =
                deployment_plan::deployment_chain(request.environment, request.grants, &scope)?;
            (ProviderInner::Chain(chain), Vec::new())
        }
        single @ (AzureCredentialType::ClientSecretCredential
        | AzureCredentialType::CertificateCredential
        | AzureCredentialType::UsernamePasswordCredential
        | AzureCredentialType::ManagedIdentityCredential
        | AzureCredentialType::WorkloadIdentityCredential) => {
            let grant =
                single_plan::single_grant(single, request.environment, request.grants, &scope)?;
            (ProviderInner::Single(grant), Vec::new())
        }
    };
    Ok(AzureTokenProvider {
        credential,
        scope,
        warnings,
        inner,
    })
}

pub struct TryDefaultCredential<'grants, F: Grants> {
    pub provider: Option<AzureTokenProvider<'grants, F>>,
    pub failure: Option<DefaultChainError>,
}

// the boundary catches construction only: the returned provider's token errors still surface to the caller
pub fn try_default_credential<'grants, F>(
    scope: &'grants str,
    environment: &'grants ChainEnvironment,
    grants: &'grants F,
) -> TryDefaultCredential<'grants, F>
where
    F: Grants + 'grants,
{
    tracing::debug!("Attempting to use DefaultAzureCredential for Azure Auth");
    let request = ProviderRequest {
        scope: Some(scope),
        credential: Some(AzureCredentialType::DefaultAzureCredential),
        environment,
        grants,
    };
    match get_azure_ad_token_provider(&request) {
        Ok(provider) => {
            tracing::debug!(
                "Successfully obtained Azure AD token provider using DefaultAzureCredential"
            );
            TryDefaultCredential {
                provider: Some(provider),
                failure: None,
            }
        }
        Err(failure) => {
            tracing::debug!("DefaultAzureCredential failed: {failure}");
            TryDefaultCredential {
                provider: None,
                failure: Some(failure),
            }
        }
    }
}
