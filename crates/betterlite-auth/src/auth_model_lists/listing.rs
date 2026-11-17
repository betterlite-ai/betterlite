use betterlite_config::types::Config;

use crate::auth_model_lists::complete_list::{ListingRequest, get_complete_model_list};
use crate::auth_model_lists::credential::CredentialSnapshot;
use crate::auth_model_lists::providers::ProviderModels;
use crate::auth_model_lists::wildcard::{DeploymentLookup, ExpansionContext, WildcardError};
pub struct ListingService {
    proxy_model_list: Vec<String>,
    // B061 fills this once credential storage is loaded; until then hydration finds no entry.
    credentials: CredentialSnapshot,
}
impl ListingService {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self {
            proxy_model_list: config
                .model_list
                .iter()
                .map(|deployment| deployment.model_name.clone())
                .collect(),
            credentials: CredentialSnapshot::default(),
        }
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn proxy_model_list(&self) -> &[String] {
        &self.proxy_model_list
    }

    pub fn authorized_models(
        &self,
        request: &ListingRequest<'_>,
        providers: &dyn ProviderModels,
        deployments: Option<&dyn DeploymentLookup>,
    ) -> Result<Vec<String>, WildcardError> {
        let context = ExpansionContext {
            providers,
            credentials: &self.credentials,
            deployments,
        };
        get_complete_model_list(&self.proxy_model_list, request, &context)
    }
}
