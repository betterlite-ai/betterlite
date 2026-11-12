#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::helpers::{HelperRun, run_case};

#[rstest::rstest]
#[case::bedrock_checks_validator("bedrock-checks-validator")]
#[case::bedrock_streaming_from_extras("bedrock-streaming-from-extras")]
#[case::provider_config_defaults("provider-config-defaults")]
#[case::stream_sink_and_key("stream-sink-and-key")]
#[case::user_metadata_transform("user-metadata-transform")]
#[case::user_metadata_merge("user-metadata-merge")]
#[case::lakera_v2_on_flagged("lakera-v2-on-flagged")]
#[case::pillar_flags_and_defaults("pillar-flags-and-defaults")]
#[case::noma_block_failures("noma-block-failures")]
#[case::zscaler_user_api_key_fields("zscaler-user-api-key-fields")]
fn cases_match_litellm(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/guardrail-provider-options")
        .join(format!("{name}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let result = run_case(&case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}

#[test]
fn replays_the_guardrail_provider_options_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("guardrail-provider-options-"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            crate::case::helper::validate_helper(&path, &case).unwrap();
        })
        .count();
    assert!(replayed >= 1);
}
