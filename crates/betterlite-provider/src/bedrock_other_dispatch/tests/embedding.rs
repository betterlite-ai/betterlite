use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::bedrock_embedding_io::response::EmbeddingReply;
use betterlite_bedrock::embedding_call::{EmbeddingCallResponse, EmbeddingEnvironment};
use betterlite_models::decisions::Json;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::Json as PromptJson;

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

// Titan G1 and V2 answer a float vector, and V2 also answers an encoded binary one
async fn titan_vectors() {
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
        }],
        "the non-async embedding leaf asks for the sync client"
    );

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

pub(super) async fn case() {
    titan_vectors().await;
    sequential_successes().await;
    sequential_failure().await;
    cohere_input_type().await;
    nova_text_payload().await;
    nova_media_payloads().await;
    marengo_payloads().await;
    async_status().await;
    extra_headers_forwarded().await;
}
