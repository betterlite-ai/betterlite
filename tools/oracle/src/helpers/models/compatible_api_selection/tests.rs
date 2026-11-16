#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_models::compatible_api_selection::{MESSAGES_ENDPOINT, RESPONSES_ENDPOINT};
use serde_json::{Value, json};

use super::{Family, Input, Op};
use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

fn answer(calls: &Value) -> Value {
    let input: Input = serde_json::from_value(json!({ "calls": calls })).unwrap();
    Op::run(&input).unwrap()
}

fn seeds() -> Vec<(String, Input)> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("compatible_api_selection"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            assert_eq!(case.call.operation, "compatible_api_selection");
            validate_helper(Path::new("<helper>"), &case).unwrap();
            let input: Input = serde_json::from_value(case.call.input).unwrap();
            Op::run(&input).unwrap();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            (name, input)
        })
        .collect()
}

fn seed<'seeds>(seeds: &'seeds [(String, Input)], marker: &str) -> &'seeds Input {
    seeds
        .iter()
        .find_map(|(name, input)| name.contains(marker).then_some(input))
        .expect("a seed whose name carries the marker")
}

#[test]
fn helper_seeds_select_compatible_api_selection() {
    let seeds = seeds();
    let collision = seed(&seeds, "native-collision");
    assert!(collision.calls.iter().any(|call| {
        matches!(call.family, Family::Responses)
            && call.native.is_some()
            && call.providers.iter().any(|descriptor| {
                Some(descriptor.provider) == call.native
                    && descriptor
                        .supported_endpoints
                        .iter()
                        .any(|endpoint| endpoint == RESPONSES_ENDPOINT)
            })
    }));
    let unsupported = seed(&seeds, "unsupported-endpoint");
    assert!(!unsupported.calls.is_empty());
    assert!(
        unsupported
            .calls
            .iter()
            .all(|call| call.providers.iter().all(|descriptor| descriptor
                .supported_endpoints
                .iter()
                .all(|endpoint| endpoint != RESPONSES_ENDPOINT && endpoint != MESSAGES_ENDPOINT)))
    );
}

#[test]
fn a_native_answer_beats_a_registry_descriptor_that_lists_responses() {
    let report = answer(&json!([
        {
            "family": "responses",
            "provider": "sail",
            "identity": "name",
            "native": "sail",
            "providers": [{"provider": "sail", "supported_endpoints": ["/v1/responses"]}]
        },
        {
            "family": "responses",
            "provider": "sail",
            "identity": "enum",
            "native": "sail",
            "providers": [{"provider": "sail", "supported_endpoints": ["/v1/responses"]}]
        }
    ]));
    assert_eq!(report, json!(["native", "native"]));
}

#[test]
fn a_responses_descriptor_without_a_native_answer_reports_the_json_slug() {
    let report = answer(&json!([{
        "family": "responses",
        "provider": "sail",
        "identity": "name",
        "native": null,
        "providers": [{"provider": "sail", "supported_endpoints": ["/v1/responses"]}]
    }]));
    assert_eq!(report, json!(["json:sail"]));
}

#[test]
fn an_unsupported_response_endpoint_selects_nothing() {
    let report = answer(&json!([{
        "family": "responses",
        "provider": "groq",
        "identity": "name",
        "native": null,
        "providers": [{"provider": "groq", "supported_endpoints": ["/v1/responses/v2"]}]
    }]));
    assert_eq!(report, json!([null]));
}

#[test]
fn an_unknown_enum_identity_is_invalid_input() {
    let input = json!({ "calls": [{
        "family": "responses",
        "provider": "not-a-provider",
        "identity": "enum"
    }] });
    let error = Op::decode_and_run(&input).unwrap_err();
    assert_eq!(error.class, "InvalidInput");
}

#[test]
fn deepseek_and_the_json_messages_tail_report_their_config_classes() {
    let report = answer(&json!([
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
        },
        {
            "family": "messages",
            "provider": "meta",
            "model": "meta-llama",
            "providers": [{"provider": "meta", "supported_endpoints": ["/v1/chat/completions"]}]
        }
    ]));
    assert_eq!(
        report,
        json!([
            "DeepSeekAnthropicMessagesConfig",
            "JSONProviderAnthropicMessagesConfig",
            null
        ])
    );
}

#[test]
fn the_embedding_rerank_and_image_arms_report_their_config_classes() {
    let report = answer(&json!([
        {"family": "embedding", "provider": "cometapi", "model": "m"},
        {"family": "embedding", "provider": "openai", "model": "m"},
        {"family": "rerank", "provider": "deepinfra", "model": "m"},
        {"family": "rerank", "provider": "cohere", "model": "m"},
        {"family": "image_generation", "provider": "aiml", "model": "m"},
        {"family": "image_generation", "provider": "cometapi", "model": "m"},
        {"family": "image_generation", "provider": "groq", "model": "m"}
    ]));
    assert_eq!(
        report,
        json!([
            "CometAPIEmbeddingConfig",
            null,
            "DeepinfraRerankConfig",
            null,
            "AimlImageGenerationConfig",
            "CometAPIImageGenerationConfig",
            null
        ])
    );
}
