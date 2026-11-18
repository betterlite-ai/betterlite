use betterlite_prompts::arguments::json::Json;

use super::super::fixture::{Probe, Reply, fixture_probe, object, run};
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
    let mut outcome_probe = fixture_probe();
    outcome_probe.model = String::from(NOVA_MODEL);
    outcome_probe.optional_params = options;
    outcome_probe.replies = vec![Reply::ok("{\"embeddings\":[{\"embedding\":[1,2,3,4]}]}")];
    let outcome = run(&outcome_probe);
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
    assert_eq!(
        vector(&reply.data[0].embedding),
        [1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64]
    );
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert_eq!(reply.usage.total_tokens, 1);
}

fn async_wraps_the_model_input() {
    let mut options = credentials();
    options.push(("output_s3_uri", "\"s3://bucket/out\""));
    let mut outcome_probe = fixture_probe();
    outcome_probe.model = String::from("async_invoke/amazon.nova-2-multimodal-embed-v1");
    outcome_probe.optional_params = options;
    outcome_probe.aembedding = true;
    outcome_probe.replies = vec![Reply::ok(
        "{\"invocationArn\":\"arn:aws:bedrock:us-east-1:123456789012:async-invoke/abc\"}",
    )];
    let outcome = run(&outcome_probe);
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
    let mut outcome_probe = fixture_probe();
    outcome_probe.model = String::from("async_invoke/amazon.nova-2-multimodal-embed-v1");
    outcome_probe.optional_params = credentials();
    let outcome = run(&outcome_probe);
    assert_eq!(
        outcome.error().to_string(),
        "output_s3_uri is required for async invoke requests"
    );
    assert_eq!(outcome.error().status(), None);
    assert!(outcome.factory_requests.is_empty());
}

fn endpoint_resolution_prefers_the_caller() {
    from_api_base_wins();
    from_the_configured_option_wins();
    from_the_environment_wins();
}

fn from_api_base_wins() {
    let outcome = run(&nova_probe(
        configured(),
        Some("https://proxy.example.test/"),
        "https://env.example.test",
    ));
    assert_eq!(
        outcome.posted().url,
        "https://proxy.example.test//model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
}

fn from_the_configured_option_wins() {
    let outcome = run(&nova_probe(configured(), None, "https://env.example.test"));
    assert_eq!(
        outcome.posted().url,
        "https://configured.example.test/model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
    assert!(
        !outcome
            .remaining
            .contains_key("aws_bedrock_runtime_endpoint")
    );
}

fn from_the_environment_wins() {
    let outcome = run(&nova_probe(credentials(), None, "https://env.example.test"));
    assert_eq!(
        outcome.posted().url,
        "https://env.example.test/model/amazon.nova-2-multimodal-embed-v1/invoke"
    );
}

fn nova_probe(params: Params, api_base: Option<&str>, runtime_endpoint: &str) -> Probe {
    let mut probe = fixture_probe();
    probe.model = String::from(NOVA_MODEL);
    probe.api_base = api_base.map(String::from);
    probe.runtime_endpoint = Some(String::from(runtime_endpoint));
    probe.optional_params = params;
    probe.replies = vec![Reply::ok("{\"embedding\":[1,2,3,4]}")];
    probe
}

type Params = Vec<(&'static str, &'static str)>;

fn configured() -> Params {
    let mut params = credentials();
    params.push((
        "aws_bedrock_runtime_endpoint",
        "\"https://configured.example.test\"",
    ));
    params
}

fn marengo_builds_its_flat_body() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.model = String::from("twelvelabs.marengo-embed-2-7-v1:0");
    outcome_probe.optional_params = credentials();
    outcome_probe.replies = vec![Reply::ok("{\"data\":[{\"embedding\":[0.5,0.25]}]}")];
    let outcome = run(&outcome_probe);
    assert_eq!(
        outcome.body(),
        object("{\"inputType\":\"text\",\"inputText\":\"hello\",\"textTruncate\":\"end\"}")
    );
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/twelvelabs.marengo-embed-2-7-v1%3A0/invoke"
    );
    let reply = batch(outcome.response());
    assert_eq!(vector(&reply.data[0].embedding), [0.5_f64, 0.25_f64]);
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
    let mut rejected_probe = fixture_probe();
    rejected_probe.model = String::from("twelvelabs.marengo-embed-3-0-v1:0");
    rejected_probe.optional_params = options.clone();
    let rejected = run(&rejected_probe);
    assert!(matches!(rejected.error(), EmbeddingCallError::Marengo(_)));
    assert_eq!(rejected.error().status(), Some(400));
    assert!(rejected.factory_requests.is_empty());
    let mut dropped_probe = fixture_probe();
    dropped_probe.model = String::from("twelvelabs.marengo-embed-3-0-v1:0");
    dropped_probe.optional_params = options;
    dropped_probe.drop_params = true;
    dropped_probe.replies = vec![Reply::ok("{\"data\":[{\"embedding\":[0.5]}]}")];
    let dropped = run(&dropped_probe);
    assert_eq!(
        dropped.body(),
        object("{\"inputType\":\"text\",\"text\":{\"inputText\":\"hello\"}}")
    );
    assert_eq!(
        vector(&batch(dropped.response()).data[0].embedding),
        [0.5_f64]
    );
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
