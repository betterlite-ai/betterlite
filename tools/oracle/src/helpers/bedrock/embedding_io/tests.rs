#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::input::Input;

#[test]
fn helper_seeds_select_bedrock_embedding_io() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_embedding_io"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert!(!seeds.is_empty(), "no bedrock_embedding_io seed exists");
    let functions: BTreeSet<&str> = seeds
        .iter()
        .map(|source| {
            let case: HelperCase =
                serde_saphyr::from_str(source).expect("a seed parses as a helper case");
            let input: Input = serde_json::from_value(case.call.input.clone())
                .expect("the seed input has the typed shape");
            assert!(
                case.call.operation == "bedrock_embedding_io"
                    && validate_helper(Path::new("<helper>"), &case).is_ok(),
                "the seed must call the bedrock_embedding_io operation"
            );
            function_name(&input)
        })
        .collect();
    let expected: BTreeSet<&str> = [
        "async_embedding_single_arn",
        "embedding_credentials",
        "embedding_usage",
        "http_error",
        "provider_json_response",
        "signed_embedding_request",
    ]
    .into_iter()
    .collect();
    assert_eq!(functions, expected, "every operation needs a seed");
}

const fn function_name(input: &Input) -> &'static str {
    match input {
        Input::SignedEmbeddingRequest(_) => "signed_embedding_request",
        Input::ProviderJsonResponse(_) => "provider_json_response",
        Input::EmbeddingUsage(_) => "embedding_usage",
        Input::HttpError(_) => "http_error",
        Input::EmbeddingCredentials(_) => "embedding_credentials",
        Input::AsyncEmbeddingSingleArn(_) => "async_embedding_single_arn",
    }
}
