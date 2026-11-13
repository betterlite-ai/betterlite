pub mod cache;
pub mod environment;
pub mod listing;
pub mod parameters;

mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::provider_metadata_factory::selection::{ProviderModelMetadata, select};

use cache::AvailableModelsCache;
use environment::EnvironmentSnapshot;
use listing::{ListingCredentials, ModelListing};
use parameters::LiteLlmParams;

pub const AZURE_MODEL: &str = "Azure-LLM";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnumerationWarning {
    ErrorGettingValidModels,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Enumeration {
    pub models: Vec<String>,
    pub warnings: Vec<EnumerationWarning>,
}

pub struct EnumerationRequest<'request> {
    pub check_provider_endpoint: bool,
    pub global_check_provider_endpoint: bool,
    pub custom_llm_provider: Option<&'request str>,
    pub parameters: LiteLlmParams,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub provider_list: &'request [ProviderId],
    pub models_by_provider: &'request BTreeMap<ProviderId, Vec<String>>,
    pub environment: &'request EnvironmentSnapshot,
}

struct ProviderAccess<'access, L> {
    cache: &'access mut AvailableModelsCache,
    models_by_provider: &'access BTreeMap<ProviderId, Vec<String>>,
    environment: &'access EnvironmentSnapshot,
    listing: &'access L,
}

impl<L> ProviderAccess<'_, L>
where
    L: ModelListing,
{
    fn models_for(
        &mut self,
        provider: ProviderId,
        adapter: &ProviderModelMetadata,
        parameters: &LiteLlmParams,
    ) -> Result<Vec<String>, EnumerationWarning> {
        let environment = self.environment;
        let cached = match self
            .cache
            .get(Some(provider), Some(parameters), environment)
        {
            Ok(cached) => cached,
            Err(error) => {
                tracing::warn!(%error, "Error getting valid models");
                return Err(EnumerationWarning::ErrorGettingValidModels);
            }
        };
        if let Some(models) = cached {
            return Ok(models);
        }
        let credentials = ListingCredentials {
            api_key: parameters.api_key.as_ref(),
            api_base: parameters.api_base.as_deref(),
        };
        let fetched = match self.listing.get_models(adapter, credentials) {
            Ok(models) => models,
            Err(error) => {
                tracing::warn!(%error, "Error getting valid models");
                return Err(EnumerationWarning::ErrorGettingValidModels);
            }
        };
        if let Err(error) = self.cache.set(provider, Some(parameters), &fetched) {
            tracing::warn!(%error, "Error getting valid models");
            return Err(EnumerationWarning::ErrorGettingValidModels);
        }
        Ok(fetched)
    }
}

#[must_use]
pub fn get_valid_models<L>(
    request: &EnumerationRequest<'_>,
    cache: &mut AvailableModelsCache,
    listing: &L,
) -> Enumeration
where
    L: ModelListing,
{
    let Some(providers) = valid_providers(request) else {
        tracing::warn!("Error getting valid models: the custom provider is not a known provider");
        return Enumeration {
            models: Vec::new(),
            warnings: vec![EnumerationWarning::ErrorGettingValidModels],
        };
    };
    let effective = effective_parameters(request);
    let check = request.check_provider_endpoint || request.global_check_provider_endpoint;
    let mut access = ProviderAccess {
        cache,
        models_by_provider: request.models_by_provider,
        environment: request.environment,
        listing,
    };
    let mut models = Vec::new();
    let mut warnings = Vec::new();
    for provider in providers {
        let selected = select(None, provider);
        if provider == ProviderId::Azure {
            models.push(AZURE_MODEL.to_owned());
        } else if let Some(adapter) = selected.filter(|_adapter| check) {
            match access.models_for(provider, &adapter, &effective) {
                Ok(provider_models) => models.extend(provider_models),
                Err(warning) => warnings.push(warning),
            }
        } else {
            models.extend(
                access
                    .models_by_provider
                    .get(&provider)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
    }
    Enumeration { models, warnings }
}

fn effective_parameters(request: &EnumerationRequest<'_>) -> LiteLlmParams {
    let mut effective = request.parameters.clone();
    if let Some(api_key) = &request.api_key {
        effective.api_key = Some(api_key.clone());
    }
    if let Some(api_base) = &request.api_base {
        effective.api_base = Some(api_base.clone());
    }
    effective
}

fn valid_providers(request: &EnumerationRequest<'_>) -> Option<Vec<ProviderId>> {
    request.custom_llm_provider.map_or_else(
        || {
            Some(infer_valid_providers(
                request.provider_list,
                request.environment,
            ))
        },
        |name| {
            name.parse::<ProviderId>()
                .ok()
                .map(|provider| vec![provider])
        },
    )
}

fn infer_valid_providers(
    provider_list: &[ProviderId],
    environment: &EnvironmentSnapshot,
) -> Vec<ProviderId> {
    let mut providers = Vec::new();
    for provider in provider_list {
        let name = provider.to_string();
        let plain = name.replace('_', "");
        let plain_key = format!("{}_API_KEY", plain.to_uppercase());
        let key = format!("{}_API_KEY", name.to_uppercase());
        if environment.names.contains(&plain_key) || environment.names.contains(&key) {
            providers.push(*provider);
        }
    }
    providers
}
