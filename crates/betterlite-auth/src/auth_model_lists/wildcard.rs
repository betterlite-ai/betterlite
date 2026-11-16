use std::collections::BTreeSet;

use crate::auth_model_lists::credential::{CredentialSnapshot, hydrate_credential_name};
use crate::auth_model_lists::parameters::{ModelParameters, ParameterError};
use crate::auth_model_lists::providers::{
    ProviderModels, get_provider_models, is_declared_provider,
};
use crate::auth_model_patterns::pattern::is_wildcard_pattern;
// B302's router facts: the deployments that answer a model name, parsed into their litellm_params.
pub trait DeploymentLookup: Send + Sync {
    fn model_rows(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<Vec<ModelParameters>, WildcardError>;
}
#[derive(Debug, thiserror::Error)]
pub enum WildcardError {
    #[error("the router's model list could not be read")]
    Lookup,
    #[error(transparent)]
    Parameters(#[from] ParameterError),
}
pub struct ExpansionContext<'context> {
    pub providers: &'context dyn ProviderModels,
    pub credentials: &'context CredentialSnapshot,
    pub deployments: Option<&'context dyn DeploymentLookup>,
}
#[must_use]
pub fn get_known_models_from_wildcard(
    wildcard_model: &str,
    parameters: Option<&ModelParameters>,
    context: &ExpansionContext<'_>,
) -> Vec<String> {
    let model_to_expand = wildcard_to_expand(wildcard_model, parameters);
    let Some((wildcard_provider_prefix, wildcard_suffix)) = model_to_expand.split_once('/') else {
        return Vec::new();
    };
    let provider = requested_provider(parameters, wildcard_provider_prefix);
    let hydrated = hydrate_credential_name(parameters, context.credentials);
    let Some(wildcard_models) = get_provider_models(provider, hydrated.as_ref(), context.providers)
    else {
        return Vec::new();
    };
    apply_suffix(wildcard_models, wildcard_suffix)
        .into_iter()
        .map(|model| prefixed(&model, wildcard_provider_prefix))
        .collect()
}
pub fn wildcard_models(
    unique_models: &mut Vec<String>,
    return_wildcard_routes: bool,
    team_id: Option<&str>,
    context: &ExpansionContext<'_>,
) -> Result<Vec<String>, WildcardError> {
    let mut models_to_remove: BTreeSet<String> = BTreeSet::new();
    let mut all_wildcard_models: Vec<String> = Vec::new();
    for model in unique_models.iter() {
        if !is_wildcard_pattern(model) {
            continue;
        }
        if return_wildcard_routes {
            all_wildcard_models.push(model.clone());
        }
        models_to_remove.insert(model.clone());
        let rows = match context.deployments {
            Some(lookup) => lookup.model_rows(model, team_id)?,
            None => Vec::new(),
        };
        if rows.is_empty() {
            all_wildcard_models.extend(get_known_models_from_wildcard(model, None, context));
            continue;
        }
        for parameters in &rows {
            all_wildcard_models.extend(get_known_models_from_wildcard(
                model,
                Some(parameters),
                context,
            ));
        }
    }
    unique_models.retain(|model| !models_to_remove.contains(model));
    Ok(all_wildcard_models)
}
// "*" expands the deployment's own provider wildcard when that wildcard carries a slash.
fn wildcard_to_expand<'model>(
    wildcard_model: &'model str,
    parameters: Option<&'model ModelParameters>,
) -> &'model str {
    match parameters {
        Some(found)
            if wildcard_model == "*"
                && is_wildcard_pattern(found.model())
                && found.model().contains('/') =>
        {
            found.model()
        }
        _ => wildcard_model,
    }
}
fn requested_provider<'parameters>(
    parameters: Option<&'parameters ModelParameters>,
    wildcard_provider_prefix: &'parameters str,
) -> &'parameters str {
    match parameters {
        Some(found) => found
            .model()
            .split_once('/')
            .map_or(found.model(), |(head, _suffix)| head),
        None => wildcard_provider_prefix,
    }
}
// a partial suffix (`gemini-*`) filters by its prefix, or prepends it when no model carries it
fn apply_suffix(wildcard_models: Vec<String>, wildcard_suffix: &str) -> Vec<String> {
    if wildcard_suffix == "*" {
        return wildcard_models;
    }
    let model_prefix = wildcard_suffix.replace('*', "");
    if wildcard_models
        .iter()
        .any(|model| model.starts_with(&model_prefix))
    {
        return wildcard_models
            .into_iter()
            .filter(|model| model.starts_with(&model_prefix))
            .collect();
    }
    wildcard_models
        .into_iter()
        .map(|model| format!("{model_prefix}{model}"))
        .collect()
}
// a known provider's leading segment is replaced, an organization segment is kept under the prefix
fn prefixed(model: &str, wildcard_provider_prefix: &str) -> String {
    if model.starts_with(wildcard_provider_prefix) {
        return model.to_owned();
    }
    match model.split_once('/') {
        Some((leading, model_suffix)) if is_declared_provider(leading) => {
            format!("{wildcard_provider_prefix}/{model_suffix}")
        }
        _ => format!("{wildcard_provider_prefix}/{model}"),
    }
}
