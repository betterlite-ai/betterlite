use std::fmt::Debug;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::resolve::split_model;
use pcre2::bytes::RegexBuilder;

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
        let split = split_model(self.catalog?, model);
        Some(IdentifiedModel {
            provider: split.provider?,
            model: split.model,
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
    // re.match starts at the subject start, and its `$` also accepts one trailing newline
    let converted = allowed_model_pattern.replace('*', ".*");
    let expression =
        format!("(*UTF)(*UCP)(*LIMIT_MATCH=1000000)(*LIMIT_DEPTH=256)\\A(?:^{converted}$)");
    let Ok(pattern) = RegexBuilder::new().build(&expression) else {
        return false;
    };
    // a match that runs past the limits is reported as an error, and the pattern then grants nothing
    matches!(pattern.is_match(model.as_bytes()), Ok(true))
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
