#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::input::Input;

#[test]
fn helper_seeds_select_bedrock_other_embedding_call() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_other_embedding_call"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert!(
        !seeds.is_empty(),
        "no bedrock_other_embedding_call seed exists"
    );
    let variants: BTreeSet<&str> = seeds
        .iter()
        .map(|source| {
            let case: HelperCase =
                serde_saphyr::from_str(source).expect("a seed parses as a helper case");
            let input: Input = serde_json::from_value(case.call.input.clone())
                .expect("the seed input has the typed shape");
            assert!(
                case.call.operation == "bedrock_other_embedding_call"
                    && validate_helper(Path::new("<helper>"), &case).is_ok(),
                "the seed must call the bedrock_other_embedding_call operation"
            );
            variant_name(&input)
        })
        .collect();
    let expected: BTreeSet<&str> = [
        "embedding_family_selection",
        "ordered_multi_input_result",
        "cohere_batched_input",
        "titan_per_item_input",
        "nova_async_invoke_start",
        "signing_auth_isolation",
        "embedding_transport_error",
    ]
    .into_iter()
    .collect();
    assert_eq!(variants, expected, "every case contract needs a seed");
}

const fn variant_name(input: &Input) -> &'static str {
    match input {
        Input::EmbeddingFamilySelection(_) => "embedding_family_selection",
        Input::OrderedMultiInputResult(_) => "ordered_multi_input_result",
        Input::CohereBatchedInput(_) => "cohere_batched_input",
        Input::TitanPerItemInput(_) => "titan_per_item_input",
        Input::NovaAsyncInvokeStart(_) => "nova_async_invoke_start",
        Input::SigningAuthIsolation(_) => "signing_auth_isolation",
        Input::EmbeddingTransportError(_) => "embedding_transport_error",
    }
}
