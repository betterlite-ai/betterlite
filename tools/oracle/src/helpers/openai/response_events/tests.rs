#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

use super::Call;

const VARIANTS: [&str; 11] = [
    "created_at_conversion",
    "lax_response_validation",
    "missing_error_code",
    "unknown_generic_event",
    "completed_incomplete_failed",
    "last_terminal_wins",
    "no_terminal_event",
    "non_native_streaming_model",
    "known_event_variants",
    "responses_auth_headers",
    "responses_base_url",
];

#[test]
fn helper_seeds_select_openai_response_events() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_response_events"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(
        !seeds.is_empty(),
        "the openai_response_events seeds are missing"
    );
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        let fields = case.call.input.as_object().unwrap();
        let variant = fields.keys().next().unwrap();
        case.call.operation == "openai_response_events"
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
