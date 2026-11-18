#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

// every seed names its variant, so a seed that starts selecting another one fails here
const SEEDS: [(&str, &str); 9] = [
    ("openai_output_limits-drop-http.yaml", "drop"),
    ("openai_output_limits-drop-sdk.yaml", "drop"),
    ("openai_output_limits-drop-malformed.yaml", "drop"),
    ("openai_output_limits-marker.yaml", "marker"),
    ("openai_output_limits-build-completion.yaml", "build"),
    ("openai_output_limits-build-stream.yaml", "build"),
    ("openai_output_limits-azure-reply.yaml", "azure"),
    ("openai_output_limits-azure-status.yaml", "azure"),
    ("openai_output_limits-azure-timeout.yaml", "azure"),
];

fn expected_variant(name: &str) -> Option<&'static str> {
    SEEDS
        .iter()
        .find_map(|(file, variant)| (*file == name).then_some(*variant))
}

// one corpus file parses as a valid helper case and selects the variant its name promises
fn checked_seed(corpus: &Path, name: &str) {
    let expected = expected_variant(name).unwrap_or_else(|| panic!("unlisted seed {name}"));
    let source = fs::read_to_string(corpus.join(name)).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let variant = case.call.input.as_object().unwrap().keys().next().unwrap();
    assert_eq!(variant, expected, "{name}");
    assert_eq!(case.call.operation, "openai_output_limits", "{name}");
    assert!(
        validate_helper(Path::new("<helper>"), &case).is_ok(),
        "{name}"
    );
}

#[test]
fn helper_seeds_select_openai_output_limits() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .filter(|name| name.starts_with("openai_output_limits"))
                .map(str::to_owned)
        })
        .inspect(|name| checked_seed(&corpus, name))
        .collect();
    assert_eq!(seeds.len(), SEEDS.len(), "{seeds:?}");
}

// every case file compares the production helper with the pinned LiteLLM
#[rstest]
fn matches_pinned_openai_output_limits(
    #[base_dir = "../../.."]
    #[files("cases/helpers/providers/openai-output-limits/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}
