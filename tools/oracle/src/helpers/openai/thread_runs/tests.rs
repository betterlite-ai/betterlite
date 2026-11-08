#![cfg(test)]

use crate::case::{HelperCase, helper::validate_helper};
use std::{fs, path::Path};

// Each seed is named after the variant it must select: openai_thread_runs-<variant with hyphens>.yaml.
fn selected_variant(source: &str) -> String {
    let case: HelperCase = serde_saphyr::from_str(source).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    assert_eq!(case.call.operation, "openai_thread_runs");
    let input = case.call.input.as_object().unwrap();
    assert_eq!(input.len(), 1);
    input.keys().next().unwrap().replace('_', "-")
}

#[test]
fn helper_seeds_select_openai_thread_runs() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("openai_thread_runs-")?;
            Some((stem.to_owned(), fs::read_to_string(&path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no openai_thread_runs seeds");
    let (stems, variants): (Vec<String>, Vec<String>) = seeds
        .into_iter()
        .map(|(stem, source)| (stem, selected_variant(&source)))
        .unzip();
    assert_eq!(stems, variants);
}
