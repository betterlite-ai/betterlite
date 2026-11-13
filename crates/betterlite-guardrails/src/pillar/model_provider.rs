use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};
use secrecy::SecretString;

use super::error::PillarError;

const UNKNOWN: &str = "unknown";

// get_llm_provider, injected so the guardrail reads no process-global catalog (B174)
pub trait ModelProviderResolver {
    fn model_and_provider(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<(String, String), ResolutionError>;
}

// The caller owns the resolver's tables, as the straiker webhook guardrail arranges
pub struct CatalogResolver<'context> {
    pub catalog: &'context ModelCatalog,
    pub known_model_names: &'context BTreeSet<String>,
    pub custom_providers: &'context BTreeSet<String>,
    pub environment: &'context BTreeMap<String, String>,
    pub registry: &'context JSONProviderRegistry,
}

impl ModelProviderResolver for CatalogResolver<'_> {
    fn model_and_provider(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<(String, String), ResolutionError> {
        let api_key = api_key.map(|text| SecretString::from(text.to_owned()));
        let request = ResolutionRequest {
            model,
            custom_llm_provider,
            api_base,
            api_key: api_key.as_ref(),
        };
        let context = ResolutionContext {
            catalog: self.catalog,
            known_model_names: self.known_model_names,
            custom_providers: self.custom_providers,
            environment: self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        resolve(&request, &context)
            .map(|resolved| (resolved.model, resolved.provider.slug().to_owned()))
    }
}

pub(super) fn optional_text(
    data: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<Option<String>, PillarError> {
    match data.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PillarError::Field {
            field,
            expected: "a string",
        }),
    }
}

fn non_empty(value: String) -> String {
    if value.is_empty() {
        UNKNOWN.to_owned()
    } else {
        value
    }
}

pub fn extract_model_and_provider(
    data: &BTreeMap<String, GuardrailValue>,
    resolver: &dyn ModelProviderResolver,
) -> Result<(String, String), PillarError> {
    let model = match optional_text(data, "model")? {
        Some(model) if !model.is_empty() => model,
        None | Some(_) => return Ok((UNKNOWN.to_owned(), UNKNOWN.to_owned())),
    };
    let custom = optional_text(data, "custom_llm_provider")?;
    let api_base = optional_text(data, "api_base")?;
    let api_key = optional_text(data, "api_key")?;
    match resolver.model_and_provider(
        &model,
        custom.as_deref(),
        api_base.as_deref(),
        api_key.as_deref(),
    ) {
        Ok((resolved, provider)) => Ok((non_empty(resolved), non_empty(provider))),
        Err(_) => {
            let declared = custom
                .filter(|value| !value.is_empty())
                .or(optional_text(data, "provider")?);
            Ok((non_empty(model), non_empty(declared.unwrap_or_default())))
        }
    }
}
