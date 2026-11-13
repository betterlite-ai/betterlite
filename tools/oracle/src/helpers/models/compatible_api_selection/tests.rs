#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::{Call, Op};
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

fn answer(calls: Value) -> Value {
    let input: Vec<Call> = serde_json::from_value(calls).unwrap();
    Op::run(&input).unwrap()
}

#[test]
fn helper_seeds_select_compatible_api_selection() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("compatible_api_selection"))
        })
        .collect();
    assert!(!seeds.is_empty());
    let names: Vec<String> = seeds
        .iter()
        .filter_map(|path| Some(path.file_name()?.to_str()?.to_owned()))
        .collect();
    assert!(names.iter().any(|name| name.contains("native-collision")));
    assert!(
        names
            .iter()
            .any(|name| name.contains("unsupported-endpoint"))
    );
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert_eq!(case.call.operation, "compatible_api_selection");
        validate_helper(Path::new("<helper>"), &case).unwrap();
        Op::decode_and_run(&case.call.input).is_ok()
    }));
}

#[test]
fn a_native_answer_beats_a_registry_descriptor_that_lists_responses() {
    let report = answer(json!([{
        "family": "responses",
        "provider": "openai",
        "identity": "name",
        "native": "openai",
        "providers": [{"provider": "openai", "supported_endpoints": ["/v1/responses"]}]
    }]));
    assert_eq!(report, json!(["native"]));
}

#[test]
fn an_unsupported_response_endpoint_selects_nothing() {
    let report = answer(json!([{
        "family": "responses",
        "provider": "groq",
        "identity": "name",
        "native": null,
        "providers": [{"provider": "groq", "supported_endpoints": ["/v1/responses/v2"]}]
    }]));
    assert_eq!(report, json!([null]));
}

#[test]
fn deepseek_and_the_json_messages_tail_report_their_config_classes() {
    let report = answer(json!([
        {
            "family": "messages",
            "provider": "deepseek",
            "model": "deepseek-chat",
            "providers": [{"provider": "deepseek", "supported_endpoints": ["/v1/messages"]}]
        },
        {
            "family": "messages",
            "provider": "groq",
            "model": "groq-llama",
            "providers": [{"provider": "groq", "supported_endpoints": ["/v1/messages"]}]
        }
    ]));
    assert_eq!(
        report,
        json!([
            "DeepSeekAnthropicMessagesConfig",
            "JSONProviderAnthropicMessagesConfig"
        ])
    );
}

#[test]
fn the_embedding_rerank_and_image_arms_report_their_config_classes() {
    let report = answer(json!([
        {"family": "embedding", "provider": "cometapi", "model": "m"},
        {"family": "rerank", "provider": "deepinfra", "model": "m"},
        {"family": "image_generation", "provider": "aiml", "model": "m"},
        {"family": "image_generation", "provider": "cometapi", "model": "m"}
    ]));
    assert_eq!(
        report,
        json!([
            "CometAPIEmbeddingConfig",
            "DeepinfraRerankConfig",
            "AimlImageGenerationConfig",
            "CometAPIImageGenerationConfig"
        ])
    );
}
