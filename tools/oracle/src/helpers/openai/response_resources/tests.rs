#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

use super::Call;

const VARIANTS: [&str; 10] = [
    "encoded_response_id",
    "delete_result",
    "get_strict_response",
    "list_default_limit_order",
    "include_comma_query",
    "cancel_response",
    "compact_keeps_query",
    "compact_replay_input",
    "compact_lax_response",
    "resource_json_error_status",
];

#[test]
fn helper_seeds_select_openai_response_resources() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_response_resources"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(
        !seeds.is_empty(),
        "the openai_response_resources seeds are missing"
    );
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        let fields = case.call.input.as_object().unwrap();
        let variant = fields.keys().next().unwrap();
        case.call.operation == "openai_response_resources"
            && fields.len() == 1
            && VARIANTS.contains(&variant.as_str())
            && serde_json::from_value::<Call>(case.call.input.clone()).is_ok_and(|call| {
                serde_json::to_value(call)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .is_some_and(|round_trip| round_trip.keys().next() == Some(variant))
            })
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
