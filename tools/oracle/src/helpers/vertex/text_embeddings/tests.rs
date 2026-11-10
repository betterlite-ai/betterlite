#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_text_embeddings"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn seed_cases() -> Vec<HelperCase> {
    seed_sources()
        .iter()
        .map(|source| serde_saphyr::from_str(source).unwrap())
        .collect()
}

fn case_models(case: &HelperCase) -> Vec<String> {
    let mut models = Vec::new();
    for key in ["requests", "replies"] {
        let Some(entries) = case.call.input.get(key).and_then(|value| value.as_array()) else {
            continue;
        };
        for entry in entries {
            if let Some(model) = entry.get("model").and_then(|value| value.as_str()) {
                models.push(model.to_owned());
            }
        }
    }
    models
}

#[test]
fn helper_seeds_select_vertex_text_embeddings() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    assert!(sources.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "vertex_text_embeddings"
    }));
}

#[test]
fn helper_seeds_cover_every_family() {
    let models: Vec<String> = seed_cases().iter().flat_map(case_models).collect();
    assert!(!models.is_empty());
    assert!(
        models
            .iter()
            .any(|model| model.bytes().all(|byte| byte.is_ascii_digit()))
    );
    assert!(
        models
            .iter()
            .any(|model| model.to_lowercase().contains("bge"))
    );
    assert!(models.iter().any(|model| {
        !model.bytes().all(|byte| byte.is_ascii_digit()) && !model.to_lowercase().contains("bge")
    }));
}
