#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

#[test]
fn helper_seeds_select_anthropic_capabilities() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("anthropic_capabilities"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!sources.is_empty(), "no anthropic_capabilities seeds");
    sources.sort();
    assert!(sources.iter().all(|source| {
        let parsed = serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|case| {
                validate_helper(Path::new("<helper>"), &case).map_err(|error| error.to_string())
            });
        parsed.is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "anthropic_capabilities"
    }));
}
