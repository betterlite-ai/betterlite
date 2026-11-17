#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

// every seed names its variant, so a seed that starts selecting another one fails here
const SEEDS: [(&str, &str); 8] = [
    ("openai_output_limits-drop-http.yaml", "drop"),
    ("openai_output_limits-drop-sdk.yaml", "drop"),
    ("openai_output_limits-drop-malformed.yaml", "drop"),
    ("openai_output_limits-marker.yaml", "marker"),
    ("openai_output_limits-build-completion.yaml", "build"),
    ("openai_output_limits-build-stream.yaml", "build"),
    ("openai_output_limits-azure-reply.yaml", "azure"),
    ("openai_output_limits-azure-timeout.yaml", "azure"),
];

fn expected_variant(name: &str) -> Option<&'static str> {
    SEEDS
        .iter()
        .find_map(|(file, variant)| (*file == name).then_some(*variant))
}

#[test]
fn helper_seeds_select_openai_output_limits() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let mut seeds = Vec::new();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("seed name");
        if !name.starts_with("openai_output_limits") {
            continue;
        }
        let expected = expected_variant(name).unwrap_or_else(|| panic!("unlisted seed {name}"));
        let source = fs::read_to_string(&path).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        let variant = case.call.input.as_object().unwrap().keys().next().unwrap();
        assert_eq!(variant, expected, "{name}");
        assert_eq!(case.call.operation, "openai_output_limits", "{name}");
        assert!(
            validate_helper(Path::new("<helper>"), &case).is_ok(),
            "{name}"
        );
        seeds.push(name.to_owned());
    }
    assert_eq!(seeds.len(), SEEDS.len(), "{seeds:?}");
}
