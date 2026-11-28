use std::time::Duration;

use betterlite_bedrock::rerank::plan::RerankEnvironment;
use betterlite_bedrock::transcription::error::CallError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankRequest};
use indexmap::IndexMap;

use super::fixture::{
    Binding, Fixture, Handler, Outcome, array, number, object, params, run, value,
};
use crate::bedrock_other_dispatch::call::{BedrockOtherOperation, RerankOperation};
use crate::bedrock_other_dispatch::error::BedrockOtherError;
use crate::bedrock_other_dispatch::test_support::client::Reply;
use crate::bedrock_other_dispatch::test_support::factory::{ClientCall, Kind};
use crate::bedrock_other_dispatch::test_support::{NOW_SECONDS, credentials, instant};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const MODEL_ARN: &str = "arn:aws:bedrock:us-east-1:123456789012:rerank-model/test";
const GENERATED_ID: &str = "00000000-0000-4000-8000-000000000000";
const CREDENTIALS: &str = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"}"#;
const ANONYMOUS_REPLY: &str =
    r#"{"results": [{"index": 1, "relevanceScore": 0.75}, {"index": 0, "relevanceScore": 0.25}]}"#;

fn scripted(replies: &[Reply]) -> Fixture {
    Fixture::new(replies, credentials(ACCESS_KEY, SECRET_KEY))
}

fn documents(count: usize) -> Vec<RerankDocument> {
    (0..count)
        .map(|index| RerankDocument::Text(format!("document {index}")))
        .collect()
}

fn request(model: &str, top_n: Option<i64>, documents: Vec<RerankDocument>) -> RerankRequest {
    RerankRequest {
        model: model.to_owned(),
        query: String::from("what is rust"),
        top_n,
        documents,
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
        truncate_prompt_tokens: None,
        truncation_side: None,
        max_tokens_per_query: None,
    }
}

fn operation<'call>(
    request: RerankRequest,
    optional: &'call str,
    api_base: Option<&'call str>,
) -> BedrockOtherOperation<'call, 'call, 'call, Handler, Binding> {
    BedrockOtherOperation::Rerank(RerankOperation {
        request,
        optional_params: params(optional),
        api_base,
        extra_headers: None,
        api_key: None,
        timeout: None,
        is_async: false,
        environment: RerankEnvironment::default(),
        now: instant(NOW_SECONDS).expect("the fixture instant"),
        generated_id: GENERATED_ID,
    })
}

fn rerank_call<'call>(request: RerankRequest, optional: &'call str) -> RerankOperation<'call> {
    RerankOperation {
        request,
        optional_params: params(optional),
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        is_async: false,
        environment: RerankEnvironment::default(),
        now: instant(NOW_SECONDS).expect("the fixture instant"),
        generated_id: GENERATED_ID,
    }
}

fn field<'json>(body: &'json Json, name: &str) -> &'json Json {
    object(body).get(name).expect("the body holds the key")
}

fn number_of_results(body: &Json) -> i64 {
    let configuration = object(field(body, "rerankingConfiguration"));
    let bedrock = object(
        configuration
            .get("bedrockRerankingConfiguration")
            .expect("the bedrock reranking configuration"),
    );
    number(bedrock.get("numberOfResults").expect("the result count"))
        .as_i64()
        .expect("the result count is an integer")
}

// The model arn names the agent-runtime endpoint and signs with the runtime's bedrock service
async fn agent_runtime_and_documents() {
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let mut documented = request(
        MODEL_ARN,
        Some(2),
        vec![
            RerankDocument::Text(String::from("first")),
            RerankDocument::Object(IndexMap::from([(
                String::from("title"),
                Json::Text(String::from("second")),
            )])),
        ],
    );
    documented.return_documents = Some(true);
    let outcome = run(&fixture, operation(documented, CREDENTIALS, None)).await;
    agent_runtime_request(&outcome);
    agent_runtime_reply(&outcome);
}

fn agent_runtime_request(outcome: &Outcome<CallError>) {
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-agent-runtime.us-east-1.amazonaws.com/rerank"
    );
    let authorization = outcome
        .authorization()
        .expect("the rerank request is signed");
    assert!(authorization.starts_with("AWS4-HMAC-SHA256"));
    assert!(authorization.contains("/bedrock/aws4_request"));
    assert!(outcome.header("x-amz-date").is_some());
    assert_eq!(
        outcome.body(0),
        value(
            r#"{"queries": [{"textQuery": {"text": "what is rust"}, "type": "TEXT"}], "rerankingConfiguration": {"bedrockRerankingConfiguration": {"modelConfiguration": {"modelArn": "arn:aws:bedrock:us-east-1:123456789012:rerank-model/test"}, "numberOfResults": 2}, "type": "BEDROCK_RERANKING_MODEL"}, "sources": [{"inlineDocumentSource": {"textDocument": {"text": "first"}, "type": "TEXT"}, "type": "INLINE"}, {"inlineDocumentSource": {"jsonDocument": {"title": "second"}, "type": "JSON"}, "type": "INLINE"}]}"#
        )
    );
}

fn agent_runtime_reply(outcome: &Outcome<CallError>) {
    let response = outcome.rerank();
    assert_eq!(response.id.as_deref(), Some(GENERATED_ID));
    let results = response
        .results
        .as_ref()
        .expect("the reply carries results");
    assert_eq!(results[0].index, 1);
    assert_eq!(results[1].index, 0);
    assert_eq!(results[0].relevance_score.0.to_string(), "0.75");
    assert!(results.iter().all(|result| result.document.is_none()));
    assert_eq!(
        response
            .meta
            .as_ref()
            .and_then(|meta| meta.billed_units.as_ref())
            .and_then(|units| units.as_ref())
            .and_then(|units| units.search_units),
        Some(Some(1)),
        "an absent usage takes the default search unit"
    );
    assert!(outcome.post_call());
    assert_eq!(
        outcome.phases,
        [String::from("pre_api_call"), String::from("post_api_call")],
        "the sync rerank logs its request and its reply"
    );
}

// top_n picks the result count and both absent and zero fall back to the document count
async fn top_n_and_region() {
    for (top_n, expected) in [(None, 3_i64), (Some(0), 3), (Some(1), 1)] {
        let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
        let outcome = run(
            &fixture,
            operation(request(MODEL_ARN, top_n, documents(3)), CREDENTIALS, None),
        )
        .await;
        assert_eq!(
            number_of_results(&outcome.body(0)),
            expected,
            "top_n {top_n:?}"
        );
        assert_eq!(array(field(&outcome.body(0), "sources")).len(), 3);
    }

    let identity = r#"{"aws_access_key_id": "AKIAIOSFODNN7EXAMPLE", "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "aws_region_name": "eu-west-1"}"#;
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let outcome = run(
        &fixture,
        operation(request("rerank-model", None, documents(2)), identity, None),
    )
    .await;
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-agent-runtime.eu-west-1.amazonaws.com/rerank"
    );
}

// The reply keeps its id and indices, and return_documents changes neither the body nor the results
async fn reply_shape() {
    let fixture = scripted(&[Reply::status(
        200,
        r#"{"id": "reply-id", "results": [{"index": 2, "relevanceScore": 0.125}, {"index": 0, "relevanceScore": -0.5}], "usage": {"search_units": 5, "total_tokens": 9, "input_tokens": 1, "output_tokens": 2}}"#,
    )]);
    let mut documented = request(MODEL_ARN, Some(2), documents(2));
    documented.return_documents = Some(true);
    let outcome = run(&fixture, operation(documented, CREDENTIALS, None)).await;
    assert_eq!(
        object(&outcome.body(0)).keys().cloned().collect::<Vec<_>>(),
        vec!["queries", "rerankingConfiguration", "sources"]
    );
    let response = outcome.rerank();
    assert_eq!(response.id.as_deref(), Some("reply-id"));
    let results = response
        .results
        .as_ref()
        .expect("the reply carries results");
    assert_eq!(results[0].index, 2);
    assert_eq!(results[1].index, 0);
    assert!(results.iter().all(|result| result.document.is_none()));
    let meta = response.meta.as_ref().expect("the reply carries meta");
    let billed = meta
        .billed_units
        .as_ref()
        .and_then(|units| units.as_ref())
        .expect("the reply carries billed units");
    assert_eq!(billed.search_units, Some(Some(5)));
    assert_eq!(billed.total_tokens, Some(Some(9)));
    let tokens = meta
        .tokens
        .as_ref()
        .and_then(|counts| counts.as_ref())
        .expect("the reply carries tokens");
    assert_eq!(tokens.input_tokens, Some(Some(1)));
    assert_eq!(tokens.output_tokens, Some(Some(2)));
}

// A provider status outside 2xx reaches the caller as the rerank arm's own failure
async fn provider_failure() {
    let fixture = scripted(&[Reply::status(400, r#"{"message": "the model refused"}"#)]);
    let outcome = run(
        &fixture,
        operation(request(MODEL_ARN, None, documents(1)), CREDENTIALS, None),
    )
    .await;
    assert!(matches!(outcome.error(), BedrockOtherError::Rerank(_)));
    assert_eq!(outcome.error().status(), Some(400));
    assert_eq!(outcome.requests.len(), 1);
}

// The caller's extra headers reach the signed agent-runtime request
async fn extra_headers_forwarded() {
    let headers = vec![(String::from("x-trace"), String::from("t1"))];
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let outcome = run(
        &fixture,
        BedrockOtherOperation::Rerank(RerankOperation {
            request: request(MODEL_ARN, None, documents(1)),
            optional_params: params(CREDENTIALS),
            api_base: None,
            extra_headers: Some(headers.as_slice()),
            api_key: None,
            timeout: None,
            is_async: false,
            environment: RerankEnvironment::default(),
            now: instant(NOW_SECONDS).expect("the fixture instant"),
            generated_id: GENERATED_ID,
        }),
    )
    .await;
    assert_eq!(outcome.header("x-trace"), Some("t1"));
}

// api_base replaces the agent-runtime host the prepared request is posted to
async fn api_base_endpoint() {
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let mut call = rerank_call(request(MODEL_ARN, None, documents(1)), CREDENTIALS);
    call.api_base = Some("https://proxy.example/base");
    let outcome = run(&fixture, BedrockOtherOperation::Rerank(call)).await;
    assert_eq!(outcome.posted().url, "https://proxy.example/base/rerank");
}

// the caller timeout reaches the posted request as its deadline
async fn timeout_deadline() {
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let mut call = rerank_call(request(MODEL_ARN, None, documents(1)), CREDENTIALS);
    call.timeout = Some(Duration::from_millis(250));
    let outcome = run(&fixture, BedrockOtherOperation::Rerank(call)).await;
    assert_eq!(outcome.posted().deadline, Some(Duration::from_millis(250)));
}

// is_async posts through the async client and skips the sync-only reply log
async fn async_rerank() {
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let mut call = rerank_call(request(MODEL_ARN, None, documents(1)), CREDENTIALS);
    call.is_async = true;
    let outcome = run(&fixture, BedrockOtherOperation::Rerank(call)).await;
    assert_eq!(
        outcome.calls,
        [ClientCall {
            kind: Kind::Async,
            provider: None,
            timeout: None,
        }],
        "the async rerank asks for the async client"
    );
    assert!(
        !outcome.post_call(),
        "the async rerank does not log the raw reply"
    );
    assert_eq!(
        outcome.phases,
        [String::from("pre_api_call")],
        "the async rerank logs only its request"
    );
    let results = outcome
        .rerank()
        .results
        .as_ref()
        .expect("the reply carries results");
    assert_eq!(results.len(), 2);
}

// a non-default environment runtime endpoint replaces the regional agent-runtime host
async fn environment_endpoint() {
    let fixture = scripted(&[Reply::status(200, ANONYMOUS_REPLY)]);
    let mut call = rerank_call(request(MODEL_ARN, None, documents(1)), CREDENTIALS);
    call.environment.runtime_endpoint = Some("https://environment.example");
    let outcome = run(&fixture, BedrockOtherOperation::Rerank(call)).await;
    assert_eq!(outcome.posted().url, "https://environment.example/rerank");
}

pub(super) async fn case() {
    agent_runtime_and_documents().await;
    top_n_and_region().await;
    reply_shape().await;
    provider_failure().await;
    extra_headers_forwarded().await;
    api_base_endpoint().await;
    timeout_deadline().await;
    async_rerank().await;
    environment_endpoint().await;
}
