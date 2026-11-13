#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

use super::input::{Input, ProviderName};

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_cache_lookup"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn seed_inputs() -> Vec<Input> {
    seed_sources()
        .iter()
        .map(|source| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            serde_json::from_value(case.call.input).unwrap()
        })
        .collect()
}

#[test]
fn helper_seeds_select_vertex_cache_lookup() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "vertex_cache_lookup"
    }));
}

#[test]
fn helper_seeds_cover_every_target_form() {
    let targets: Vec<_> = seed_inputs()
        .into_iter()
        .flat_map(|input| input.targets)
        .collect();
    let has = |wanted: ProviderName| targets.iter().any(|case| case.provider == wanted);
    assert!(has(ProviderName::VertexAi));
    assert!(has(ProviderName::VertexAiBeta));
    assert!(has(ProviderName::Gemini));
    assert!(targets.iter().any(|case| {
        case.api_base
            .as_deref()
            .is_some_and(|base| base.ends_with("/v1") || base.ends_with("/v1beta1"))
    }));
    assert!(targets.iter().any(|case| {
        case.api_base
            .as_deref()
            .is_some_and(|base| base.contains('?'))
    }));
}
