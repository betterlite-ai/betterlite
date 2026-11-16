use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::types::Config;
use betterlite_models::model_registration::JsonObject;

use crate::auth_model_lists::complete_list::{ListingRequest, get_complete_model_list};
use crate::auth_model_lists::credential::CredentialSnapshot;
use crate::auth_model_lists::fallbacks::{FallbackGroups, get_all_fallbacks};
use crate::auth_model_lists::model_info::expand_wildcard_deployments_for_model_info;
use crate::auth_model_lists::providers::ProviderModels;
use crate::auth_model_lists::wildcard::{DeploymentLookup, ExpansionContext, WildcardError};

// The config facts a model listing reads: the proxy's model names, its fallback plans and its credentials.
pub struct ListingService {
    proxy_model_list: Vec<String>,
    fallbacks: Settings,
    credentials: CredentialSnapshot,
}

impl ListingService {
    #[must_use]
    pub fn new(
        proxy_model_list: Vec<String>,
        fallbacks: Settings,
        credentials: CredentialSnapshot,
    ) -> Self {
        Self {
            proxy_model_list,
            fallbacks,
            credentials,
        }
    }

    // the constructor the server state builds once; B061 later supplies the loaded credentials.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self::new(
            config
                .model_list
                .iter()
                .map(|deployment| deployment.model_name.clone())
                .collect(),
            config.sections.fallback_plans.clone(),
            CredentialSnapshot::default(),
        )
    }

    #[must_use]
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

    #[must_use]
    pub fn expand_model_info(
        &self,
        deployments: &[JsonObject],
        providers: &dyn ProviderModels,
    ) -> Vec<JsonObject> {
        let context = ExpansionContext {
            providers,
            credentials: &self.credentials,
            deployments: None,
        };
        expand_wildcard_deployments_for_model_info(deployments, &context)
    }

    // a listing with no router has no fallback plans to read, matching a call with llm_router None.
    #[must_use]
    pub fn fallbacks_for(
        &self,
        model: &str,
        fallback_type: &str,
        groups: Option<&dyn FallbackGroups>,
    ) -> Vec<String> {
        match groups {
            Some(resolver) => get_all_fallbacks(model, fallback_type, Some(&self.fallbacks), resolver),
            None => Vec::new(),
        }
    }
}
