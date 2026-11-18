use betterlite_prompts::arguments::json::Json;

use super::super::fixture::{Probe, Reply, object, run};
use super::super::support::{batch, titan_body};
use crate::embedding_call::error::EmbeddingCallError;

pub(super) fn embedding_family_selection() {
    provider_markers();
    unknown_provider_names_the_model();
    async_marker_is_removed_once();
    model_id_is_quoted();
    regions_follow_credentials();
    options_are_copied_not_shared();
    bearer_token_skips_credentials();
}

fn provider_markers() {
    let nova = run(&Probe {
        model: String::from("amazon.nova-2-multimodal-embed-v1"),
        replies: vec![Reply::ok("{\"embeddings\":[{\"embedding\":[0.5]}]}")],
        ..Probe::default()
    });
    assert!(nova.body().contains_key("schemaVersion"));
    let marengo = run(&Probe {
        model: String::from("twelvelabs.marengo-embed-2-7-v1:0"),
        replies: vec![Reply::ok("{\"data\":[{\"embedding\":[0.5]}]}")],
        ..Probe::default()
    });
    assert_eq!(
        marengo.body().get("inputType"),
        Some(&Json::Text(String::from("text")))
    );
    let titan = run(&Probe {
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(titan.body(), object("{\"inputText\":\"hello\"}"));
}

fn unknown_provider_names_the_model() {
    let outcome = run(&Probe {
        model: String::from("anthropic.claude-3"),
        ..Probe::default()
    });
    assert_eq!(
        outcome.error().to_string(),
        "Unable to determine bedrock embedding provider for model: anthropic.claude-3. Supported providers: ['cohere', 'amazon', 'twelvelabs', 'nova']"
    );
    assert_eq!(outcome.regions, ["us-east-1"]);
    assert!(outcome.factory_requests.is_empty());
    assert!(outcome.cohere_requests.is_empty());
    assert!(outcome.supplied_requests.is_empty());
}

fn async_marker_is_removed_once() {
    let stripped = run(&Probe {
        model: String::from("async_invoke/amazon.titan-embed-text-v1"),
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(
        stripped.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/async-invoke"
    );
    assert_eq!(
        batch(stripped.response()).model,
        "amazon.titan-embed-text-v1"
    );
    let doubled = run(&Probe {
        model: String::from("async_invoke/async_invoke/amazon.titan-embed-text-v1"),
        ..Probe::default()
    });
    assert!(matches!(doubled.error(), EmbeddingCallError::Unmapped));
    assert!(doubled.factory_requests.is_empty());
}

fn model_id_is_quoted() {
    let mut options = credential_options();
    options.push(("model_id", "\"custom/model id\""));
    let outcome = run(&Probe {
        optional_params: options,
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/custom%2Fmodel%20id/invoke"
    );
    assert_eq!(
        batch(outcome.response()).model,
        "amazon.titan-embed-text-v1"
    );
    assert!(!outcome.remaining.contains_key("model_id"));
    let mut empty_options = credential_options();
    empty_options.push(("model_id", "\"\""));
    let empty = run(&Probe {
        optional_params: empty_options,
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert!(
        empty
            .posted()
            .url
            .ends_with("/model/amazon.titan-embed-text-v1/invoke")
    );
}

fn regions_follow_credentials() {
    let explicit = run(&Probe {
        environment: vec![("AWS_REGION", "eu-west-1")],
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(explicit.regions, ["us-east-1"]);
    assert!(explicit.posted().url.contains("bedrock-runtime.us-east-1."));
    let environment = run(&Probe {
        optional_params: identity_options_without_region(),
        environment: vec![
            ("AWS_REGION_NAME", "ap-south-1"),
            ("AWS_REGION", "eu-west-1"),
        ],
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(environment.regions, ["eu-west-1"]);
    assert!(
        environment
            .posted()
            .url
            .contains("bedrock-runtime.eu-west-1.")
    );
    let default = run(&Probe {
        optional_params: identity_options_without_region(),
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(default.regions, ["us-west-2"]);
}

fn options_are_copied_not_shared() {
    let mut options = vec![
        ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
        ("AWS_ACCESS_KEY_ID", "\"LEFTOVER\""),
        (
            "aws_secret_access_key",
            "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
        ),
        ("aws_region_name", "\"us-east-1\""),
        (
            "aws_bedrock_runtime_endpoint",
            "\"https://runtime.example.test\"",
        ),
    ];
    options.push(("dimensions", "512"));
    options.push(("user", "\"alice\""));
    options.push(("User", "\"bob\""));
    let outcome = run(&Probe {
        model: String::from("amazon.titan-embed-text-v2:0"),
        optional_params: options,
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(
        outcome.body(),
        object("{\"inputText\":\"hello\",\"User\":\"bob\",\"dimensions\":512}")
    );
    assert_eq!(
        outcome.posted().url,
        "https://runtime.example.test/model/amazon.titan-embed-text-v2%3A0/invoke"
    );
    for key in [
        "aws_access_key_id",
        "aws_secret_access_key",
        "aws_region_name",
        "aws_bedrock_runtime_endpoint",
    ] {
        assert!(!outcome.remaining.contains_key(key));
    }
    assert!(outcome.remaining.contains_key("user"));
    assert!(outcome.remaining.contains_key("AWS_ACCESS_KEY_ID"));
}

fn bearer_token_skips_credentials() {
    let configured = run(&Probe {
        bearer: Some(String::from("configured-token")),
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(
        configured.headers()["Authorization"],
        "Bearer configured-token"
    );
    assert!(configured.regions.is_empty());
    assert!(!configured.headers().contains_key("X-Amz-Date"));
    assert!(!configured.headers().contains_key("X-Amz-Security-Token"));
    let caller = run(&Probe {
        api_key: Some(String::from("caller-token")),
        bearer: Some(String::from("configured-token")),
        replies: vec![Reply::ok(&titan_body("[0.5]", 1))],
        ..Probe::default()
    });
    assert_eq!(caller.headers()["Authorization"], "Bearer caller-token");
}

fn credential_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
        (
            "aws_secret_access_key",
            "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
        ),
        ("aws_region_name", "\"us-east-1\""),
    ]
}

fn identity_options_without_region() -> Vec<(&'static str, &'static str)> {
    vec![
        ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
        (
            "aws_secret_access_key",
            "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
        ),
    ]
}
