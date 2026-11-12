#![cfg(test)]

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

fn seed_sources() -> Vec<(PathBuf, String)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources = Vec::new();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("multimodal_embedding_records") {
            let source = fs::read_to_string(&path).unwrap();
            sources.push((path, source));
        }
    }
    sources
}

fn seed_operations(sources: &[(PathBuf, String)]) -> Vec<String> {
    let mut operations = Vec::new();
    for (path, source) in sources {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(path, &case).unwrap();
        operations.push(case.call.operation);
    }
    operations
}

#[test]
fn helper_seeds_select_multimodal_embedding_records() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let operations = seed_operations(&sources);
    assert!(
        operations
            .iter()
            .all(|operation| operation == "multimodal_embedding_records")
    );
}
