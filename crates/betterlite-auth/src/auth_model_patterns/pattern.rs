use std::fmt::Debug;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::provider_id::ProviderId;
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedModel {
    pub provider: String,
    pub model: String,
}

pub trait ProviderInference: Debug + Send + Sync {
    fn identify(&self, model: &str) -> Option<IdentifiedModel>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CatalogProviderInference<'catalog> {
    pub catalog: Option<&'catalog ModelCatalog>,
}

impl ProviderInference for CatalogProviderInference<'_> {
    fn identify(&self, model: &str) -> Option<IdentifiedModel> {
        if let Some((prefix, rest)) = model.split_once('/') {
            if prefix.parse::<ProviderId>().is_err() || rest.is_empty() {
                return None;
            }
            return Some(IdentifiedModel {
                provider: prefix.to_owned(),
                model: rest.to_owned(),
            });
        }
        let catalog = self.catalog?;
        // an exact catalog entry wins; the fallback generalizations route an otherwise-unknown id
        if let Some(metadata) = catalog.metadata(model) {
            return Some(IdentifiedModel {
                provider: metadata.provider.to_owned(),
                model: model.to_owned(),
            });
        }
        let provider = catalog.generalizations.match_routing(model)?;
        Some(IdentifiedModel {
            provider: provider.to_owned(),
            model: model.to_owned(),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnresolvedProviderInference;

impl ProviderInference for UnresolvedProviderInference {
    fn identify(&self, _model: &str) -> Option<IdentifiedModel> {
        None
    }
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn is_wildcard_pattern(allowed_model_pattern: &str) -> bool {
    allowed_model_pattern.contains('*')
}

#[must_use]
pub fn is_model_allowed_by_pattern(model: &str, allowed_model_pattern: &str) -> bool {
    if !is_wildcard_pattern(allowed_model_pattern) {
        return false;
    }
    // Python's re.match with a `$` also matches before one trailing newline, so the group keeps that
    let expression = format!("^{}(?:\\n)?$", allowed_model_pattern.replace('*', ".*"));
    Regex::new(&expression).is_ok_and(|pattern| pattern.is_match(model))
}

#[must_use]
pub fn matches_wildcard_patterns(
    model: &str,
    allowed_model_list: &[String],
    inference: &dyn ProviderInference,
) -> bool {
    let direct = allowed_model_list.iter().any(|allowed_model_pattern| {
        is_wildcard_pattern(allowed_model_pattern)
            && is_model_allowed_by_pattern(model, allowed_model_pattern)
    });
    if direct {
        return true;
    }
    allowed_model_list.iter().any(|allowed_model_pattern| {
        is_wildcard_pattern(allowed_model_pattern)
            && provider_wildcard_matches(model, allowed_model_pattern, inference)
    })
}

#[must_use]
pub fn provider_wildcard_matches(
    model: &str,
    allowed_model_pattern: &str,
    inference: &dyn ProviderInference,
) -> bool {
    let Some(identified) = inference.identify(model) else {
        return false;
    };
    // a namespace get_llm_provider did not consume must never be rebuilt into a provider model
    if identified.model == model && model.contains('/') {
        return false;
    }
    is_model_allowed_by_pattern(
        &format!("{}/{}", identified.provider, identified.model),
        allowed_model_pattern,
    )
}
