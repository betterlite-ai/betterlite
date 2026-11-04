use betterlite_models::json_provider_registry::JSONProviderRegistry;
use serde::Deserialize;
use std::io;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    source: Option<String>,
    #[serde(default)]
    missing: bool,
    #[serde(default)]
    read_failure: bool,
    retry: Option<String>,
    #[serde(default)]
    base_urls: Vec<String>,
    #[serde(default)]
    slugs: Vec<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let registry = load_fixture(input);
        let entries: Vec<_> = registry.providers.values().collect();
        let by_base: Vec<_> = input
            .base_urls
            .iter()
            .map(|base| registry.get_by_base_url(base).map(|entry| entry.provider))
            .collect();
        let exists: Vec<_> = input
            .slugs
            .iter()
            .map(|slug| registry.exists(slug))
            .collect();
        let responses: Vec<_> = input
            .slugs
            .iter()
            .map(|slug| registry.supports_responses_api(slug))
            .collect();
        Ok(
            serde_json::json!({"entries": entries, "count": entries.len(), "by_base": by_base, "exists": exists, "responses": responses}),
        )
    }
}

fn load_fixture(input: &Input) -> JSONProviderRegistry {
    let mut registry = JSONProviderRegistry::default();
    if input.missing || input.read_failure {
        let kind = if input.missing {
            io::ErrorKind::NotFound
        } else {
            io::ErrorKind::Other
        };
        registry.load_from(|| Err(io::Error::from(kind)));
    } else if let Some(source) = &input.source {
        registry.load_from(|| Ok(source.as_bytes().to_vec()));
    } else {
        registry = JSONProviderRegistry::load();
    }
    if let Some(retry) = &input.retry {
        registry.load_from(|| Ok(retry.as_bytes().to_vec()));
    }
    registry
}

mod tests;
