use std::collections::BTreeSet;

use betterlite_models::catalog::ModelCatalog;

use super::query::DEFAULT_MODEL;
use super::target::ListenTarget;

pub const PROVIDER: &str = "deepgram";

const STREAMING_PREFIX: &str = "streaming/";
const MULTILINGUAL_LANGUAGE: &str = "multi";
const MULTILINGUAL_SUFFIX: &str = "-multilingual";
const ADDON_PARAMS: [(&str, &str); 5] = [
    ("redact", "redact"),
    ("keyterm", "keyterm"),
    ("detect_entities", "detect_entities"),
    ("diarize", "diarize"),
    ("diarize_model", "diarize"),
];

#[must_use]
pub fn pricing_model(target: &ListenTarget) -> String {
    let model = target.query.first("model").unwrap_or(DEFAULT_MODEL);
    let streaming = format!("{STREAMING_PREFIX}{model}");
    let language = target.query.first("language").unwrap_or_default();
    if language.trim().to_lowercase() == MULTILINGUAL_LANGUAGE {
        format!("{streaming}{MULTILINGUAL_SUFFIX}")
    } else {
        streaming
    }
}

#[must_use]
pub fn registry_key(target: &ListenTarget) -> String {
    let model = pricing_model(target);
    format!("{PROVIDER}/{model}")
}

// only a catalog name equal to the registry key counts; a missing name has no price at all
#[must_use]
pub fn is_priced(catalog: &ModelCatalog, target: &ListenTarget) -> bool {
    let key = registry_key(target);
    let Some(metadata) = catalog.metadata(&key) else {
        return false;
    };
    metadata.provider == PROVIDER
}

#[must_use]
pub fn addon_pricing_models(target: &ListenTarget) -> Vec<String> {
    let mut models = BTreeSet::new();
    for (param, addon) in ADDON_PARAMS {
        let enabled = target.query.values(param).into_iter().any(|value| {
            let normalized = value.trim().to_lowercase();
            !normalized.is_empty() && normalized != "false"
        });
        if enabled {
            models.insert(format!("{STREAMING_PREFIX}{addon}"));
        }
    }
    models.into_iter().collect()
}
