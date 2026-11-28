use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::bedrock_embedding_io::response::EmbeddingReply;
use betterlite_bedrock::bedrock_embedding_io::transport::{ASYNC_CLIENT_PROVIDER, Timeout};
use betterlite_bedrock::embedding_call::{EmbeddingCallResponse, EmbeddingEnvironment};
use betterlite_models::decisions::Json;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::Json as PromptJson;
use secrecy::SecretString;

use super::fixture::{Fixture, Handler, environment, object, params, run, value};
use crate::bedrock_other_dispatch::call::{
    BedrockOtherOperation, EmbeddingInput, EmbeddingOperation, EmbeddingStatusOperation,
};
use crate::bedrock_other_dispatch::error::BedrockOtherError;
use crate::bedrock_other_dispatch::test_support::client::Reply;
use crate::bedrock_other_dispatch::test_support::factory::{ClientCall, Kind};
use crate::bedrock_other_dispatch::test_support::{NOW_SECONDS, credentials, instant};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const TITAN: &str = "amazon.titan-embed-text-v1";
const TITAN_V2: &str = "amazon.titan-embed-text-v2:0";
const COHERE: &str = "cohere.embed-english-v3";
const NOVA: &str = "amazon.nova-2-multimodal-embed-v1";
const MARENGO: &str = "twelvelabs.marengo-embed-2-7-v1:0";
const MARENGO3: &str = "twelvelabs.marengo-embed-3-0-v1:0";
const IDENTITY: &str = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "aws_region_name": "us-east-1"}"#;
const COHERE_QUERY: &str = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "aws_region_name": "us-east-1", "input_type": "search_query"}"#;
const MARENGO_IMAGE: &str = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "aws_region_name": "us-east-1", "input_type": "image"}"#;
const MARENGO_TRUNCATE: &str = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "aws_region_name": "us-east-1", "textTruncate": "END"}"#;
const KEYS: &str = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"}"#;
const VECTOR: &str = r#"{"embedding": [0.5], "inputTextTokenCount": 1}"#;

fn scripted(replies: &[Reply]) -> Fixture {
    Fixture::new(replies, credentials(ACCESS_KEY, SECRET_KEY))
}

fn batch(response: &EmbeddingCallResponse) -> &EmbeddingReply {
    match response {
        EmbeddingCallResponse::Batch(reply) => reply,
        EmbeddingCallResponse::Legacy(_other) => panic!("the batch arm returns a batch reply"),
    }
}

fn legacy(response: &EmbeddingCallResponse) -> &LegacyEmbeddingResponse {
    match response {
        EmbeddingCallResponse::Legacy(reply) => reply,
        EmbeddingCallResponse::Batch(_other) => panic!("the legacy arm returns a legacy reply"),
    }
}

fn floats(embedding: &EmbeddingVector) -> Vec<f64> {
    let EmbeddingVector::Floats(numbers) = embedding else {
        panic!("the vector holds floats");
    };
    numbers
        .iter()
        .filter_map(serde_json::Number::as_f64)
        .collect()
}

fn legacy_floats(embedding: &PromptJson) -> Vec<f64> {
    let PromptJson::List(items) = embedding else {
        panic!("the legacy embedding is a JSON array");
    };
    let numbers: Vec<serde_json::Number> = items
        .iter()
        .filter_map(|item| match item {
            PromptJson::Number(number) => Some(number.clone()),
            PromptJson::Null
            | PromptJson::Bool(_)
            | PromptJson::Text(_)
            | PromptJson::List(_)
            | PromptJson::Object(_) => None,
        })
        .collect();
    floats(&EmbeddingVector::Floats(numbers))
}

fn operation<'call>(
    model: &'call str,
    input: EmbeddingInput<'call>,
    optional: &'call str,
    environment: &'call BTreeMap<String, String>,
) -> BedrockOtherOperation<'call, 'call, 'call, Handler, super::fixture::Binding> {
    BedrockOtherOperation::Embedding(EmbeddingOperation {
        model,
        input,
        optional_params: params(optional),
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        aembedding: false,
        drop_params: false,
        environment: EmbeddingEnvironment {
            variables: environment,
            runtime_endpoint: None,
            bearer_token: None,
        },
    })
}

fn embedding_call<'call>(
    text: &'call str,
    optional: &'call str,
    environment: &'call BTreeMap<String, String>,
) -> EmbeddingOperation<'call> {
    EmbeddingOperation {
        model: TITAN,
        input: EmbeddingInput::Text(text),
        optional_params: params(optional),
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        aembedding: false,
        drop_params: false,
        environment: EmbeddingEnvironment {
            variables: environment,
            runtime_endpoint: None,
            bearer_token: None,
        },
    }
}

// Titan G1 answers a float vector, signed at the fixture instant through the sync client
async fn titan_g1_vector() {
    let env = environment("{}");
    let single = vec![String::from("hello")];
    let g1 = scripted(&[Reply::status(
        200,
        r#"{"embedding": [0.5, 0.25], "inputTextTokenCount": 2}"#,
    )]);
    let g1_outcome = run(
        &g1,
        operation(TITAN, EmbeddingInput::Multiple(&single), IDENTITY, &env),
    )
    .await;
    let g1_reply = batch(g1_outcome.embedding());
    assert_eq!(floats(&g1_reply.data[0].embedding), [0.5_f64, 0.25_f64]);
    assert_eq!(g1_reply.data[0].index, 0);
    assert_eq!(g1_reply.usage.prompt_tokens, 2);
    assert_eq!(g1_reply.usage.total_tokens, 2);
    assert_eq!(g1_outcome.body(0), value(r#"{"inputText": "hello"}"#));
    assert!(
        g1_outcome
            .authorization()
            .is_some_and(|header| header.starts_with("AWS4-HMAC-SHA256"))
    );
    assert_eq!(g1_outcome.regions, [String::from("us-east-1")]);
    assert_eq!(
        g1_outcome.header("x-amz-date"),
        Some("20231114T221320Z"),
        "the fixture clock signs the request"
    );
    assert_eq!(
        g1_outcome.calls,
        [ClientCall {
            kind: Kind::Sync,
            provider: None,
            timeout: None,
        }],
        "the non-async embedding leaf asks for the sync client"
    );
    assert!(g1_outcome.post_call(), "the leaf logs its reply");
    assert_eq!(
        g1_outcome.phases,
        [String::from("pre_api_call"), String::from("post_api_call")],
        "one input logs its request and its reply"
    );
}

// Titan V2 answers an encoded binary vector when it is asked for one
async fn titan_v2_binary_vector() {
    let env = environment("{}");
    let single = vec![String::from("hello")];
    let binary = scripted(&[Reply::status(
        200,
        r#"{"embeddingsByType": {"binary": "QUJD"}, "inputTextTokenCount": 2}"#,
    )]);
    let binary_outcome = run(
        &binary,
        operation(TITAN_V2, EmbeddingInput::Multiple(&single), IDENTITY, &env),
    )
    .await;
    let binary_reply = batch(binary_outcome.embedding());
    assert_eq!(
        binary_reply.data[0].embedding,
        EmbeddingVector::Base64(String::from("QUJD"))
    );
    assert_eq!(binary_outcome.body(0), value(r#"{"inputText": "hello"}"#));
}

// Inputs run one request at a time, in order, and their replies are indexed in that order
async fn sequential_successes() {
    let env = environment("{}");
    let inputs = vec![
        String::from("one"),
        String::from("two"),
        String::from("three"),
    ];
    let fixture = scripted(&[
        Reply::status(200, r#"{"embedding": [1.5], "inputTextTokenCount": 3}"#),
        Reply::status(200, r#"{"embedding": [2.5], "inputTextTokenCount": 5}"#),
        Reply::status(200, r#"{"embedding": [3.5], "inputTextTokenCount": 1}"#),
    ]);
    let outcome = run(
        &fixture,
        operation(TITAN, EmbeddingInput::Multiple(&inputs), IDENTITY, &env),
    )
    .await;
    assert_eq!(outcome.requests.len(), 3);
    assert_eq!(outcome.body(0), value(r#"{"inputText": "one"}"#));
    assert_eq!(outcome.body(1), value(r#"{"inputText": "two"}"#));
    assert_eq!(outcome.body(2), value(r#"{"inputText": "three"}"#));
    let reply = batch(outcome.embedding());
    assert_eq!(reply.data.len(), 3);
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.data[2].index, 2);
    assert_eq!(floats(&reply.data[2].embedding), [3.5_f64]);
    assert_eq!(reply.usage.prompt_tokens, 9);
}

// The first failed input stops the later ones, and its provider status reaches the caller
async fn sequential_failure() {
    let env = environment("{}");
    let failing = scripted(&[
        Reply::status(200, r#"{"embedding": [1.0], "inputTextTokenCount": 1}"#),
        Reply::status(500, r#"{"message": "the model failed"}"#),
    ]);
    let inputs = vec![
        String::from("one"),
        String::from("two"),
        String::from("three"),
    ];
    let failed = run(
        &failing,
        operation(TITAN, EmbeddingInput::Multiple(&inputs), IDENTITY, &env),
    )
    .await;
    assert!(matches!(failed.error(), BedrockOtherError::Embedding(_)));
    assert_eq!(failed.error().status(), Some(500));
    assert_eq!(failed.requests.len(), 2);
    assert_eq!(
        failed.requests[1].body,
        b"{\"inputText\":\"two\"}".as_slice()
    );
}

// The Cohere family reaches the legacy transport once with every input and its own input_type
async fn cohere_input_type() {
    let env = environment("{}");
    let inputs = vec![String::from("one"), String::from("two")];
    let fixture = scripted(&[Reply::status(
        200,
        r#"{"embeddings": [[0.1, 0.2], [0.3, 0.4]], "meta": {"billed_units": {"input_tokens": 7}}}"#,
    )]);
    let outcome = run(
        &fixture,
        operation(
            COHERE,
            EmbeddingInput::Multiple(&inputs),
            COHERE_QUERY,
            &env,
        ),
    )
    .await;
    assert_eq!(outcome.requests.len(), 1);
    assert!(outcome.calls.is_empty());
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/cohere.embed-english-v3/invoke"
    );
    assert_eq!(
        outcome.body(0),
        value(r#"{"input_type": "search_query", "texts": ["one", "two"]}"#)
    );
    let reply = legacy(outcome.embedding());
    assert_eq!(reply.object, "list");
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.data[1].index, 1);
    assert_eq!(legacy_floats(&reply.data[1].embedding), [0.3_f64, 0.4_f64]);
    assert_eq!(reply.usage.prompt_tokens, 7);
    assert_eq!(reply.usage.total_tokens, 7);
    assert_eq!(
        outcome.phases,
        [String::from("pre_api_call")],
        "the legacy sync transport logs its request and not its reply"
    );
}

// Nova embeds text with its schema version, purpose and the default dimension
async fn nova_text_payload() {
    let env = environment("{}");
    let text = String::from("hello");
    let fixture = scripted(&[Reply::status(
        200,
        r#"{"embeddings": [{"embedding": [1, 2, 3, 4]}]}"#,
    )]);
    let outcome = run(
        &fixture,
        operation(NOVA, EmbeddingInput::Text(&text), IDENTITY, &env),
    )
    .await;
    assert_eq!(
        outcome.body(0),
        value(
            r#"{"schemaVersion": "nova-multimodal-embed-v1", "taskType": "SINGLE_EMBEDDING", "singleEmbeddingParams": {"embeddingPurpose": "GENERIC_INDEX", "embeddingDimension": 3072, "text": {"value": "hello", "truncationMode": "END"}}}"#
        )
    );
    let reply = batch(outcome.embedding());
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.usage.prompt_tokens, 1);
}

// A Nova data URL becomes a media entry with its own format and the bytes after the comma
async fn nova_media_payloads() {
    let env = environment("{}");
    for (url, family, bytes, media_format) in [
        ("data:image/png;base64,AAAA", "image", "AAAA", "png"),
        ("data:video/mp4;base64,BBBB", "video", "BBBB", "mp4"),
    ] {
        let media = String::from(url);
        let fixture = scripted(&[Reply::status(
            200,
            r#"{"embeddings": [{"embedding": [0.5]}]}"#,
        )]);
        let outcome = run(
            &fixture,
            operation(NOVA, EmbeddingInput::Text(&media), IDENTITY, &env),
        )
        .await;
        let body = outcome.body(0);
        let embedding_params = object(
            object(&body)
                .get("singleEmbeddingParams")
                .expect("the params"),
        );
        let entry = object(embedding_params.get(family).expect("the media entry"));
        let media_source = object(entry.get("source").expect("the media source"));
        assert_eq!(
            media_source.get("bytes"),
            Some(&Json::Text(String::from(bytes)))
        );
        assert_eq!(
            entry.get("format"),
            Some(&Json::Text(String::from(media_format)))
        );
        assert_eq!(batch(outcome.embedding()).data.len(), 1);
    }
}

// Marengo 2.7 and 3.0 name their media payload fields differently and both answer a vector
async fn marengo_payloads() {
    let env = environment("{}");
    let image = String::from("data:image/png;base64,CCCC");
    let two_seven = scripted(&[Reply::status(
        200,
        r#"{"data": [{"embedding": [0.5, 0.25]}]}"#,
    )]);
    let outcome = run(
        &two_seven,
        operation(MARENGO, EmbeddingInput::Text(&image), MARENGO_IMAGE, &env),
    )
    .await;
    assert_eq!(
        outcome.body(0),
        value(r#"{"inputType": "image", "mediaSource": {"base64String": "CCCC"}}"#)
    );
    assert_eq!(
        floats(&batch(outcome.embedding()).data[0].embedding),
        [0.5_f64, 0.25_f64]
    );

    let three = scripted(&[Reply::status(200, r#"{"data": [{"embedding": [0.5]}]}"#)]);
    let three_outcome = run(
        &three,
        operation(MARENGO3, EmbeddingInput::Text(&image), MARENGO_IMAGE, &env),
    )
    .await;
    assert_eq!(
        three_outcome.body(0),
        value(r#"{"inputType": "image", "image": {"mediaSource": {"base64String": "CCCC"}}}"#)
    );
}

// B632's status lookup reads the arn it was handed and never starts a job, reads S3 or polls
async fn async_status() {
    let fixture = scripted(&[Reply::status(
        200,
        r#"{"status": "Completed", "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": "s3://bucket/out"}}}"#,
    )]);
    let arn = "arn:aws:bedrock:us-east-1:123456789012:async-invoke/abc";
    let mut optional = params(IDENTITY);
    let now: SystemTime = instant(NOW_SECONDS).expect("the fixture instant");
    let env = environment("{}");
    let outcome = run(
        &fixture,
        BedrockOtherOperation::EmbeddingStatus(EmbeddingStatusOperation {
            invocation_arn: arn,
            region: "us-east-1",
            optional_params: &mut optional,
            environment: &env,
            now,
        }),
    )
    .await;
    assert_eq!(
        outcome.status_body(),
        &value(
            r#"{"status": "Completed", "outputDataConfig": {"s3OutputDataConfig": {"s3Uri": "s3://bucket/out"}}}"#
        )
    );
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(outcome.posted().method, "GET");
    assert!(outcome.posted().body.is_empty());
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/async-invoke/arn%3Aaws%3Abedrock%3Aus-east-1%3A123456789012%3Aasync-invoke%2Fabc"
    );
    assert_eq!(outcome.regions, [String::from("us-east-1")]);
    assert!(optional.is_empty());
    assert!(outcome.pre_call(), "the status lookup logs its request");
    assert!(outcome.post_call(), "the status lookup logs its reply");
    assert_eq!(
        outcome.phases,
        [String::from("pre_api_call"), String::from("post_api_call")],
        "the dispatcher's own context records both status phases"
    );
}

// The caller's extra headers reach the signed embedding request
async fn extra_headers_forwarded() {
    let env = environment("{}");
    let text = String::from("hello");
    let headers = vec![(String::from("x-trace"), String::from("t1"))];
    let fixture = scripted(&[Reply::status(
        200,
        r#"{"embeddings": [{"embedding": [1, 2, 3, 4]}]}"#,
    )]);
    let outcome = run(
        &fixture,
        BedrockOtherOperation::Embedding(EmbeddingOperation {
            model: NOVA,
            input: EmbeddingInput::Text(&text),
            optional_params: params(IDENTITY),
            api_base: None,
            extra_headers: Some(headers.as_slice()),
            api_key: None,
            timeout: None,
            aembedding: false,
            drop_params: false,
            environment: EmbeddingEnvironment {
                variables: &env,
                runtime_endpoint: None,
                bearer_token: None,
            },
        }),
    )
    .await;
    assert_eq!(outcome.header("x-trace"), Some("t1"));
}

// api_base or the environment's runtime endpoint replaces the host the signed request is posted to
async fn endpoint_overrides() {
    let text = String::from("hello");
    let overrides = [
        (
            Some("https://proxy.example/bedrock"),
            None,
            "https://proxy.example/bedrock/model/amazon.titan-embed-text-v1/invoke",
        ),
        (
            None,
            Some("https://environment.example"),
            "https://environment.example/model/amazon.titan-embed-text-v1/invoke",
        ),
    ];
    for (api_base, runtime_endpoint, expected) in overrides {
        let env = environment("{}");
        let fixture = scripted(&[Reply::status(200, VECTOR)]);
        let mut call = embedding_call(&text, IDENTITY, &env);
        call.api_base = api_base;
        call.environment.runtime_endpoint = runtime_endpoint;
        let outcome = run(&fixture, BedrockOtherOperation::Embedding(call)).await;
        assert_eq!(
            outcome.posted().url,
            expected,
            "overrides {api_base:?} {runtime_endpoint:?}"
        );
    }
}

// aembedding asks for the async bedrock client and hands it the caller's timeout
async fn async_client_timeout() {
    let env = environment("{}");
    let text = String::from("hello");
    let fixture = scripted(&[Reply::status(200, VECTOR)]);
    let mut call = embedding_call(&text, IDENTITY, &env);
    call.aembedding = true;
    call.timeout = Some(Timeout::Seconds(2.5));
    let outcome = run(&fixture, BedrockOtherOperation::Embedding(call)).await;
    assert_eq!(
        outcome.calls,
        [ClientCall {
            kind: Kind::Async,
            provider: Some(String::from(ASYNC_CLIENT_PROVIDER)),
            timeout: Some(Timeout::Seconds(2.5)),
        }],
        "the async branch builds its client with the caller timeout"
    );
    assert_eq!(
        floats(&batch(outcome.embedding()).data[0].embedding),
        [0.5_f64]
    );
}

// a bearer api_key replaces the signature with the token it carries
async fn bearer_api_key() {
    let env = environment("{}");
    let text = String::from("hello");
    let key = SecretString::from(String::from("bedrock-token"));
    let fixture = scripted(&[Reply::status(200, VECTOR)]);
    let mut call = embedding_call(&text, IDENTITY, &env);
    call.api_key = Some(&key);
    let outcome = run(&fixture, BedrockOtherOperation::Embedding(call)).await;
    assert_eq!(outcome.authorization(), Some("Bearer bedrock-token"));
    assert!(
        outcome.header("x-amz-date").is_none(),
        "the bearer path does not sign the request"
    );
}

// a Marengo 2.7-only parameter is refused unless drop_params removes it
async fn drop_params_forwarded() {
    let reply = r#"{"data": [{"embedding": [0.5]}]}"#;
    let env = environment("{}");
    let text = String::from("hello");
    let refused_fixture = scripted(&[Reply::status(200, reply)]);
    let mut refused = embedding_call(&text, MARENGO_TRUNCATE, &env);
    refused.model = MARENGO3;
    let failure = run(&refused_fixture, BedrockOtherOperation::Embedding(refused)).await;
    assert!(matches!(failure.error(), BedrockOtherError::Embedding(_)));
    assert!(
        failure.requests.is_empty(),
        "the refused body is never posted"
    );
    let dropped_fixture = scripted(&[Reply::status(200, reply)]);
    let mut dropped = embedding_call(&text, MARENGO_TRUNCATE, &env);
    dropped.model = MARENGO3;
    dropped.drop_params = true;
    let outcome = run(&dropped_fixture, BedrockOtherOperation::Embedding(dropped)).await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(
        floats(&batch(outcome.embedding()).data[0].embedding),
        [0.5_f64]
    );
}

// an environment region signs against that region when no optional param names one
async fn environment_region() {
    let variables = environment(r#"{"AWS_REGION": "eu-west-1"}"#);
    let text = String::from("hello");
    let fixture = scripted(&[Reply::status(200, VECTOR)]);
    let outcome = run(
        &fixture,
        BedrockOtherOperation::Embedding(embedding_call(&text, KEYS, &variables)),
    )
    .await;
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/amazon.titan-embed-text-v1/invoke"
    );
    assert_eq!(outcome.regions, [String::from("eu-west-1")]);
}

async fn payloads() {
    titan_g1_vector().await;
    titan_v2_binary_vector().await;
    sequential_successes().await;
    sequential_failure().await;
    cohere_input_type().await;
    nova_text_payload().await;
    nova_media_payloads().await;
    marengo_payloads().await;
}

async fn forwarding() {
    async_status().await;
    extra_headers_forwarded().await;
    endpoint_overrides().await;
    async_client_timeout().await;
    bearer_api_key().await;
    drop_params_forwarded().await;
    environment_region().await;
}

pub(super) async fn case() {
    payloads().await;
    forwarding().await;
}
