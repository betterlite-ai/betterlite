#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const VARIANTS: [&str; 3] = [
    "append_message",
    "list_messages",
    "map_create_message_params",
];

// Each seed is named after the variant it must select: azure_message_attachments-<variant with hyphens>.yaml.
fn selected_variant(name: &str, source: &str) -> String {
    let case: HelperCase = serde_saphyr::from_str(source).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    case.call.check(&[crate::GROUP]).unwrap();
    assert_eq!(case.call.operation, "azure_message_attachments");
    let input = case.call.input.as_object().unwrap();
    assert_eq!(input.len(), 1, "{name}");
    input.keys().next().unwrap().replace('_', "-")
}

#[test]
fn helper_seeds_select_azure_message_attachments() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("azure_message_attachments-")?;
            Some((stem.to_owned(), fs::read_to_string(&path).unwrap()))
        })
        .collect();
    assert!(!seeds.is_empty(), "no azure_message_attachments seeds");
    let variants: BTreeSet<String> = seeds
        .iter()
        .map(|(stem, source)| {
            let variant = selected_variant(stem, source);
            assert_eq!(stem, &variant);
            variant
        })
        .collect();
    assert_eq!(variants.len(), VARIANTS.len());
}
