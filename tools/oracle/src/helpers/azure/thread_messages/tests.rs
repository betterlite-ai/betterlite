#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const VARIANTS: [&str; 4] = [
    "thread-create",
    "thread-retrieve",
    "assistants-beta-default",
    "azure-thread-async-flags",
];

// Each seed is named after the variant it must select: azure_thread_messages-<variant with hyphens>.yaml.
#[test]
fn helper_seeds_select_azure_thread_messages() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("azure_thread_messages-")?;
            Some((stem.to_owned(), fs::read_to_string(&path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no azure_thread_messages seeds");
    let selected: Vec<String> = seeds
        .iter()
        .map(|(stem, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            validate_helper(Path::new("<helper>"), &case).unwrap();
            case.call.check(&[crate::GROUP]).unwrap();
            assert_eq!(case.call.operation, "azure_thread_messages", "{stem}");
            let input = case.call.input.as_object().unwrap();
            assert_eq!(input.len(), 1, "{stem}");
            let variant = input.keys().next().unwrap().replace('_', "-");
            assert_eq!(stem, &variant);
            variant
        })
        .collect();
    let missing = VARIANTS
        .into_iter()
        .position(|expected| !selected.iter().any(|variant| variant.as_str() == expected));
    assert!(missing.is_none(), "no seed selects variant {missing:?}");
}
