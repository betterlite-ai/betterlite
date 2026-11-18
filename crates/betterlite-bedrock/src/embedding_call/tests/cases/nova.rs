use betterlite_prompts::arguments::json::Json;

use super::super::fixture::{Probe, Reply, object, run};
use super::super::support::{batch, vector};
use crate::bedrock_embedding_io::response::PromptTokenCounts;
use crate::embedding_call::error::EmbeddingCallError;

const NOVA_MODEL: &str = "amazon.nova-2-multimodal-embed-v1";

pub(super) fn nova_async_invoke_start() {
    nova_sync_builds_one_request_per_input();
    async_wraps_the_model_input();
    missing_s3_uri_fails_before_the_endpoint();
    endpoint_resolution_prefers_the_caller();
    marengo_builds_its_flat_body();
    marengo_three_rejects_its_old_params();
}

fn nova_sync_builds_one_request_per_input() {
    let mut options = credentials();
    options.push(("dimensions", "1024"));
    let outcome = run(&Probe {
        model: String::from(NOVA_MODEL),
        optional_params: options,
        replies: vec![Reply::ok("{\"embeddings\":[{\"embedding\":[1,2,3,4]}]}")],
        ..Probe::default()
    });
    assert_eq!(
        outcome.body(),
        object(
            r#"{"schemaVersion":"nova-multimodal-embed-v1","taskType":"SINGLE_EMBEDDING","singleEmbeddingParams":{"embeddingPurpose":"GENERIC_INDEX","embeddingDimension":1024,"text":{"value":"hello","truncationMode":"END"}}}"#
        )
    );
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
    let reply = batch(outcome.response());
    assert_eq!(vector(&reply.data[0].embedding), [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert_eq!(reply.usage.total_tokens, 1);
}

fn async_wraps_the_model_input() {
    let mut options = credentials();
    options.push(("output_s3_uri", "\"s3://bucket/out\""));
    let outcome = run(&Probe {
        model: String::from("async_invoke/amazon.nova-2-multimodal-embed-v1"),
        optional_params: options,
        aembedding: true,
        replies: vec![Reply::ok(
            "{\"invocationArn\":\"arn:aws:bedrock:us-east-1:123456789012:async-invoke/abc\"}",
        )],
        ..Probe::default()
    });
    assert_eq!(
        outcome.body(),
        object(
            r#"{"modelId":"amazon.nova-2-multimodal-embed-v1","modelInput":{"schemaVersion":"nova-multimodal-embed-v1","taskType":"SEGMENTED_EMBEDDING","segmentedEmbeddingParams":{"embeddingPurpose":"GENERIC_INDEX","embeddingDimension":3072,"text":{"value":"hello","truncationMode":"END"}}},"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://bucket/out"}}}"#
        )
    );
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/async-invoke"
    );
    let reply = batch(outcome.response());
    assert_eq!(
        reply.invocation_arn,
        Json::Text(String::from(
            "arn:aws:bedrock:us-east-1:123456789012:async-invoke/abc"
        ))
    );
    assert_eq!(reply.usage.prompt_tokens, 0);
}

fn missing_s3_uri_fails_before_the_endpoint() {
    let outcome = run(&Probe {
        model: String::from("async_invoke/amazon.nova-2-multimodal-embed-v1"),
        optional_params: credentials(),
        ..Probe::default()
    });
    assert_eq!(
        outcome.error().to_string(),
        "output_s3_uri is required for async invoke requests"
    );
    assert_eq!(outcome.error().status(), None);
    assert!(outcome.factory_requests.is_empty());
}

fn endpoint_resolution_prefers_the_caller() {
    let mut configured = credentials();
    configured.push((
        "aws_bedrock_runtime_endpoint",
        "\"https://configured.example.test\"",
    ));
    let from_api_base = run(&Probe {
        model: String::from(NOVA_MODEL),
        api_base: Some(String::from("https://proxy.example.test/")),
        runtime_endpoint: Some(String::from("https://env.example.test")),
        optional_params: configured.clone(),
        replies: vec![Reply::ok("{\"embedding\":[1,2,3,4]}")],
        ..Probe::default()
    });
    assert_eq!(
        from_api_base.posted().url,
        "https://proxy.example.test//model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
    let from_configured = run(&Probe {
        model: String::from(NOVA_MODEL),
        runtime_endpoint: Some(String::from("https://env.example.test")),
        optional_params: configured,
        replies: vec![Reply::ok("{\"embedding\":[1,2,3,4]}")],
        ..Probe::default()
    });
    assert_eq!(
        from_configured.posted().url,
        "https://configured.example.test/model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
    assert!(
        !from_configured
            .remaining
            .contains_key("aws_bedrock_runtime_endpoint")
    );
    let from_environment = run(&Probe {
        model: String::from(NOVA_MODEL),
        runtime_endpoint: Some(String::from("https://env.example.test")),
        optional_params: credentials(),
        replies: vec![Reply::ok("{\"embedding\":[1,2,3,4]}")],
        ..Probe::default()
    });
    assert_eq!(
        from_environment.posted().url,
        "https://env.example.test/model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
}

fn marengo_builds_its_flat_body() {
    let outcome = run(&Probe {
        model: String::from("twelvelabs.marengo-embed-2-7-v1:0"),
        optional_params: credentials(),
        replies: vec![Reply::ok("{\"data\":[{\"embedding\":[0.5,0.25]}]}")],
        ..Probe::default()
    });
    assert_eq!(
        outcome.body(),
        object("{\"inputType\":\"text\",\"inputText\":\"hello\",\"textTruncate\":\"end\"}")
    );
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/twelvelabs.marengo-embed-2-7-v1%3A0/invoke"
    );
    let reply = batch(outcome.response());
    assert_eq!(vector(&reply.data[0].embedding), [0.5, 0.25]);
    assert_eq!(
        reply.usage.prompt_tokens_details,
        Some(PromptTokenCounts {
            image_count: None,
            query_count: Some(1),
        })
    );
}

fn marengo_three_rejects_its_old_params() {
    let mut options = credentials();
    options.push(("textTruncate", "\"end\""));
    let rejected = run(&Probe {
        model: String::from("twelvelabs.marengo-embed-3-0-v1:0"),
        optional_params: options.clone(),
        ..Probe::default()
    });
    assert!(matches!(rejected.error(), EmbeddingCallError::Marengo(_)));
    assert_eq!(rejected.error().status(), Some(400));
    assert!(rejected.factory_requests.is_empty());
    let dropped = run(&Probe {
        model: String::from("twelvelabs.marengo-embed-3-0-v1:0"),
        optional_params: options,
        drop_params: true,
        replies: vec![Reply::ok("{\"data\":[{\"embedding\":[0.5]}]}")],
        ..Probe::default()
    });
    assert_eq!(
        dropped.body(),
        object("{\"inputType\":\"text\",\"text\":{\"inputText\":\"hello\"}}")
    );
    assert_eq!(vector(&batch(dropped.response()).data[0].embedding), [0.5]);
}

fn credentials() -> Vec<(&'static str, &'static str)> {
    vec![
        ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
        (
            "aws_secret_access_key",
            "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
        ),
        ("aws_region_name", "\"us-east-1\""),
    ]
}
