#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const VARIANTS: [&str; 10] = [
    "plain_text_ssml_escaping",
    "speed_rate_boundaries",
    "voice_language",
    "style_role_element",
    "existing_ssml",
    "cognitive_to_tts_endpoint",
    "response_format_header",
    "binary_native_speech",
    "native_voice_map",
    "native_speech_headers",
];

#[test]
fn helper_seeds_select_azure_native_speech() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read_to_string(path).unwrap())
        })
        .filter(|(name, _source)| name.starts_with("azure_native_speech"))
        .collect();
    assert!(!seeds.is_empty(), "no azure_native_speech seeds");
    let expected: BTreeSet<String> = VARIANTS.iter().copied().map(str::to_owned).collect();
    assert_eq!(seed_variants(&seeds), expected);
}

fn seed_variants(seeds: &[(String, String)]) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for (name, source) in seeds {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        assert_eq!(case.call.operation, "azure_native_speech");
        validate_helper(Path::new("<helper>"), &case).unwrap();
        let variant = case
            .call
            .input
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        assert!(
            VARIANTS.contains(&variant.as_str()),
            "{name} names {variant}"
        );
        seen.insert(variant);
    }
    seen
}
