#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

const VARIANTS: [&str; 12] = [
    "tool_call_json_content",
    "tool_calls_finish_reason",
    "empty_choice",
    "reasoning_content_delta",
    "embedded_stream_error",
    "response_headers",
    "model_list",
    "query_url_join",
    "stream_envelope_fields",
    "chat_auth_content_type",
    "model_list_decode_failure",
    "credential_accessor_defaults",
];

#[test]
fn helper_seeds_select_openai_chat_choices() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_chat_choices"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    for source in seeds {
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert_eq!(case.call.operation, "openai_chat_choices");
        validate_helper(Path::new("<helper>"), &case).unwrap();
        let fields = case.call.input.as_object().unwrap();
        assert_eq!(fields.len(), 1, "{source}");
        assert!(
            fields.keys().any(|key| VARIANTS.contains(&key.as_str())),
            "{source}"
        );
    }
}
