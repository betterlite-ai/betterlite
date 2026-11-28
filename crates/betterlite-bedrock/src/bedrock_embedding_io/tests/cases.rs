use std::collections::BTreeMap;

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_error::error::ErrorClass;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::fixture::{
    ACCESS_KEY, Factory, FixtureClient, Reply, SECRET_KEY, SESSION_TOKEN, Source, credentials,
    environment, headers, json, now, object, raw, runtime, secret, secret_lookup,
};
use crate::aws_endpoints::provider::BedrockEmbeddingProvider;
use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::credentials::{
    CredentialRequest, LoadedCredentials, ManagedRegionWorker, load_credentials,
};
use crate::bedrock_embedding_io::error::EmbeddingIoError;
use crate::bedrock_embedding_io::response::{EmbeddingReply, transform_response};
use crate::bedrock_embedding_io::signed_get::sign_get_request;
use crate::bedrock_embedding_io::transport::{
    EmbeddingCall, EmbeddingReply as TransportReply, EmbeddingRequest, ReplyRequest, Timeout,
    TimeoutRecord, TransferError, post_embedding,
};
use crate::bedrock_settings::{ErrorHeaders, ResponseBody};
use crate::signing::request::AwsSignedRequest;

const URL: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/amazon.titan-embed-text-v1/invoke";
const SIGNED_WITH_TOKEN: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=7387881321f6a548e401a3ef6c1731c45b4c503fe6b7e0f11d11ff1df1a9f971";
const SIGNED_WITHOUT_TOKEN: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=066bd582a40b2b288970e81ffe9294daa10597bc222e0766c637854b40fd43dc";
const SIGNED_STALE_WITHOUT_TOKEN: &str = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=91238d98d50bd2b155f790c8d30b8d170837f3ae7af5bc0cd5c0026baa259d3e";
const BODY: &str = r#"{"inputText":"hello"}"#;
const TITAN_G1_REPLY: &str = r#"{"embedding":[0.1,0.2],"inputTextTokenCount":7}"#;
const MARENGO_MODEL: &str = "twelvelabs.marengo-embed-2-7-v1:0";
const NOVA_MODEL: &str = "amazon.nova-2-multimodal-embeddings-v1:0";
const COHERE_MODEL: &str = "cohere.embed-english-v3";

fn header_map(pairs: Vec<(String, String)>) -> BTreeMap<String, String> {
    pairs.into_iter().collect()
}

fn signed_request(
    credentials_source: &Credentials,
    supplied: &[(String, String)],
) -> AwsSignedRequest {
    sign_get_request(credentials_source, URL, supplied, "us-west-2", now())
        .expect("the fixture request signs")
}

fn signed_request_with_token() {
    let request = signed_request(
        &credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN)),
        &headers(&[("Content-Type", "application/json")]),
    );
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, URL);
    assert!(request.body.is_empty());
    let signed = header_map(request.headers);
    assert_eq!(signed["Authorization"], SIGNED_WITH_TOKEN);
    assert_eq!(signed["Content-Type"], "application/json");
    assert_eq!(signed["X-Amz-Date"], "20231114T221320Z");
    assert_eq!(signed["X-Amz-Security-Token"], SESSION_TOKEN);
}

fn signed_request_without_token() {
    let request = signed_request(
        &credentials(ACCESS_KEY, SECRET_KEY, None),
        &headers(&[("Content-Type", "application/json")]),
    );
    let signed = header_map(request.headers);
    assert_eq!(signed["Authorization"], SIGNED_WITHOUT_TOKEN);
    assert!(!signed.contains_key("X-Amz-Security-Token"));
    assert!(!signed.keys().any(|name| name.eq_ignore_ascii_case("host")));
    assert!(
        !signed
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    );
}

fn signed_request_replaces_stale_headers() {
    let stale = headers(&[
        ("Content-Type", "application/json"),
        ("Authorization", "AWS4-HMAC-SHA256 caller"),
        ("X-Amz-Date", "20200101T000000Z"),
        ("X-Amz-Security-Token", "stale-token"),
    ]);
    let with_token = header_map(
        signed_request(
            &credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN)),
            &stale,
        )
        .headers,
    );
    assert_eq!(with_token["Authorization"], SIGNED_WITH_TOKEN);
    assert_eq!(with_token["X-Amz-Date"], "20231114T221320Z");
    assert_eq!(with_token["X-Amz-Security-Token"], SESSION_TOKEN);
    // botocore only replaces a stale security token when the credentials carry one
    let without_token =
        header_map(signed_request(&credentials(ACCESS_KEY, SECRET_KEY, None), &stale).headers);
    assert_eq!(without_token["Authorization"], SIGNED_STALE_WITHOUT_TOKEN);
    assert_eq!(without_token["X-Amz-Security-Token"], "stale-token");
}

fn supplied_date_uses_the_shared_signer_deviation() {
    let request = signed_request(
        &credentials(ACCESS_KEY, SECRET_KEY, None),
        &headers(&[
            ("Content-Type", "application/json"),
            ("Date", "Tue, 14 Nov 2023 22:13:20 GMT"),
        ]),
    );
    let signed = header_map(request.headers);
    assert!(!signed.keys().any(|name| name.eq_ignore_ascii_case("date")));
    assert_eq!(signed["X-Amz-Date"], "20231114T221320Z");
    assert_eq!(signed["Authorization"], SIGNED_WITHOUT_TOKEN);
}

fn signed_embedding_request() {
    supplied_date_uses_the_shared_signer_deviation();
    signed_request_with_token();
    signed_request_without_token();
    signed_request_replaces_stale_headers();
}

fn embedding_call(
    asynchronous: bool,
    data: &Object,
    timeout: Option<Timeout>,
) -> EmbeddingCall<'_> {
    EmbeddingCall {
        asynchronous,
        url: URL,
        headers: &[],
        data,
        timeout,
    }
}

fn run_direct(
    asynchronous: bool,
    factory: &Factory,
    data: &Object,
    timeout: Option<Timeout>,
) -> Result<Object, EmbeddingIoError> {
    let call = embedding_call(asynchronous, data, timeout);
    runtime().block_on(post_embedding(
        &call,
        None::<SuppliedClient<'_, FixtureClient>>,
        factory,
    ))
}

fn run_with_supplied(
    factory: &Factory,
    client: &FixtureClient,
    kind: ClientKind,
    asynchronous: bool,
    data: &Object,
) -> Result<Object, EmbeddingIoError> {
    let call = embedding_call(asynchronous, data, None);
    let supplied = SuppliedClient {
        kind,
        transport: client,
    };
    runtime().block_on(post_embedding(&call, Some(supplied), factory))
}

fn request_body(request: &EmbeddingRequest) -> Json {
    serde_json::from_slice(&request.body).expect("the fixture request body is JSON")
}

fn built_sync_client_gets_the_timeout() {
    let data = object(BODY);
    let factory = Factory::new(Reply::ok(r#"{"embeddings":[]}"#));
    let reply = run_direct(false, &factory, &data, Some(Timeout::Seconds(2.5)))
        .expect("the fixture reply decodes");
    assert_eq!(reply, object(r#"{"embeddings":[]}"#));
    assert_eq!(factory.sync_calls(), vec![Some(Timeout::Seconds(2.5))]);
    assert!(factory.async_calls().is_empty());
    let requests = factory.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, URL);
    assert_eq!(request_body(&requests[0]), json(BODY));
}

fn built_async_client_gets_the_provider_and_timeout() {
    let data = object(BODY);
    let factory = Factory::new(Reply::ok(r#"{"embeddings":[]}"#));
    let reply = run_direct(true, &factory, &data, Some(Timeout::Seconds(3.0)))
        .expect("the fixture reply decodes");
    assert_eq!(reply, object(r#"{"embeddings":[]}"#));
    assert_eq!(
        factory.async_calls(),
        vec![("bedrock".to_owned(), Some(Timeout::Seconds(3.0)))]
    );
    assert!(factory.sync_calls().is_empty());
}

fn timeout_record_is_passed_whole() {
    let data = object(BODY);
    let factory = Factory::new(Reply::ok(r#"{"embeddings":[]}"#));
    let timeout = Timeout::Record(Box::new(TimeoutRecord {
        connect: 1.0,
        read: 2.0,
        write: 3.0,
        pool: 4.0,
    }));
    run_direct(false, &factory, &data, Some(timeout.clone())).expect("the fixture reply decodes");
    assert_eq!(factory.sync_calls(), vec![Some(timeout.clone())]);
    assert_eq!(
        timeout.fields().map(f64::to_bits),
        [
            1.0_f64.to_bits(),
            2.0_f64.to_bits(),
            3.0_f64.to_bits(),
            4.0_f64.to_bits(),
        ]
    );
}

fn supplied_client_is_reused(kind: ClientKind, asynchronous: bool) {
    let data = object(BODY);
    let factory = Factory::new(Reply::ok(r#"{"factory":true}"#));
    let client = FixtureClient::new(Reply::ok(r#"{"supplied":true}"#));
    let reply = run_with_supplied(&factory, &client, kind, asynchronous, &data)
        .expect("the supplied client replies");
    assert_eq!(reply, object(r#"{"supplied":true}"#));
    assert_eq!(client.requests().len(), 1);
    assert!(factory.requests().is_empty());
    assert!(factory.sync_calls().is_empty());
    assert!(factory.async_calls().is_empty());
}

fn supplied_client_of_the_other_kind_is_not_used() {
    let data = object(BODY);
    let factory = Factory::new(Reply::ok(r#"{"factory":true}"#));
    let client = FixtureClient::new(Reply::ok(r#"{"supplied":true}"#));
    let reply = run_with_supplied(&factory, &client, ClientKind::Sync, true, &data)
        .expect("a fresh async client replies");
    assert_eq!(reply, object(r#"{"factory":true}"#));
    let second_reply = run_with_supplied(&factory, &client, ClientKind::Asynchronous, false, &data)
        .expect("a fresh sync client replies");
    assert_eq!(second_reply, object(r#"{"factory":true}"#));
    assert!(client.requests().is_empty());
    assert_eq!(factory.async_calls(), vec![("bedrock".to_owned(), None)]);
    assert_eq!(factory.sync_calls(), vec![None]);
}

fn provider_json_response() {
    built_sync_client_gets_the_timeout();
    built_async_client_gets_the_provider_and_timeout();
    timeout_record_is_passed_whole();
    supplied_client_is_reused(ClientKind::Sync, false);
    supplied_client_is_reused(ClientKind::Asynchronous, true);
    supplied_client_of_the_other_kind_is_not_used();
}

fn provider_status_is_a_bedrock_error() {
    let data = object(BODY);
    let factory = Factory::new(Reply::status(
        400,
        &[("X-Amzn-Requestid", "req-1")],
        "bad request text",
    ));
    let error = run_direct(false, &factory, &data, None).expect_err("the fixture status fails");
    assert_eq!(error.status(), Some(400));
    let EmbeddingIoError::Provider {
        status,
        error: bedrock_error,
    } = error
    else {
        panic!("a provider status becomes a bedrock error");
    };
    assert_eq!(status, 400);
    assert_eq!(bedrock_error.error.message, "bad request text");
    assert_eq!(bedrock_error.error.status_code, 400);
    assert_eq!(bedrock_error.error.response.status_code, 400);
    assert_eq!(bedrock_error.error.response.request.method, "POST");
    assert_eq!(bedrock_error.error.response.request.url, URL);
    assert_eq!(
        bedrock_error.response_headers,
        headers(&[("X-Amzn-Requestid", "req-1")])
    );
    assert_eq!(
        bedrock_error.headers,
        Some(ErrorHeaders::Http(headers(&[(
            "X-Amzn-Requestid",
            "req-1"
        )])))
    );
    assert_eq!(
        bedrock_error.response_body,
        ResponseBody::Read("bad request text".to_owned())
    );
}

fn provider_status_on_the_async_path_is_a_bedrock_error() {
    let data = object(BODY);
    let factory = Factory::new(Reply::status(503, &[], "busy"));
    let error = run_direct(true, &factory, &data, Some(Timeout::Seconds(5.0)))
        .expect_err("the fixture status fails");
    let EmbeddingIoError::Provider {
        status,
        error: bedrock_error,
    } = error
    else {
        panic!("a provider status becomes a bedrock error");
    };
    assert_eq!(status, 503);
    assert_eq!(bedrock_error.error.message, "busy");
    assert_eq!(
        factory.async_calls(),
        vec![("bedrock".to_owned(), Some(Timeout::Seconds(5.0)))]
    );
}

fn status_boundaries_are_the_success_range() {
    let data = object(BODY);
    for status in [200, 299] {
        let factory = Factory::new(Reply::status(status, &[], r#"{"embeddings":[]}"#));
        let reply = run_direct(false, &factory, &data, None).expect("a 2xx status passes through");
        assert_eq!(reply, object(r#"{"embeddings":[]}"#));
    }
    for status in [300, 399, 400, 599, 600] {
        let factory = Factory::new(Reply::status(status, &[], "failure"));
        let error = run_direct(false, &factory, &data, None)
            .expect_err("every status outside 2xx is an error");
        assert_eq!(error.status(), Some(status));
    }
}

fn assert_timeout_error(error: EmbeddingIoError, message: &str) {
    assert_eq!(error.status(), Some(408));
    let EmbeddingIoError::Handler(ErrorClass::Timeout(details)) = error else {
        panic!("the HTTP wrapper exposes the shared Timeout class");
    };
    assert_eq!(details.original_status.0, 408);
    assert_eq!(details.context.message, message);
    assert_eq!(details.context.model.as_deref(), Some("default-model-name"));
    assert_eq!(
        details.context.provider.as_deref(),
        Some("litellm-httpx-handler")
    );
    assert_eq!(details.context.body_type, None);
    assert_eq!(details.context.body_param, None);
    assert_eq!(details.context.body_code, None);
}

fn timeout_is_the_shared_handler_error() {
    let data = object(BODY);
    let sync_error = run_direct(false, &Factory::new(Reply::Timeout), &data, None)
        .expect_err("the fixture times out");
    assert_timeout_error(sync_error, "Connection timed out after None seconds.");
    let async_error = run_direct(true, &Factory::new(Reply::Timeout), &data, None)
        .expect_err("the fixture times out");
    assert_timeout_error(
        async_error,
        "Connection timed out. Timeout passed=None, time taken=0.125 seconds",
    );
}

fn transport_failure_stays_a_transport_error() {
    let data = object(BODY);
    let sync_error = run_direct(false, &Factory::new(Reply::Failed), &data, None)
        .expect_err("the fixture transport fails");
    assert!(matches!(
        sync_error,
        EmbeddingIoError::Transport(TransferError::Failed(_))
    ));
    let async_error = run_direct(true, &Factory::new(Reply::Failed), &data, None)
        .expect_err("the fixture transport fails");
    assert!(matches!(
        async_error,
        EmbeddingIoError::Transport(TransferError::Failed(_))
    ));
}

fn malformed_success_body_is_a_decode_error() {
    let data = object(BODY);
    let factory = Factory::new(Reply::ok("not json"));
    let error = run_direct(false, &factory, &data, None)
        .expect_err("a body that is not JSON fails after the status check");
    assert!(matches!(error, EmbeddingIoError::Decode { .. }));
}

fn non_utf8_error_body_names_the_byte_count() {
    let data = object(BODY);
    let reply = Reply::Answer(TransportReply {
        status: 400,
        headers: Vec::new(),
        body: vec![0xff, 0xfe],
        request: ReplyRequest {
            method: "POST".to_owned(),
            url: String::new(),
        },
    });
    let error = run_direct(false, &Factory::new(reply), &data, None)
        .expect_err("a non-UTF-8 error body is still a provider error");
    let EmbeddingIoError::Provider {
        error: bedrock_error,
        ..
    } = error
    else {
        panic!("a non-UTF-8 error body is a provider error");
    };
    assert_eq!(bedrock_error.error.message, "2 bytes that are not UTF-8");
    assert_eq!(bedrock_error.error.status_code, 400);
}

fn redirect_error_keeps_the_final_request() {
    let data = object(BODY);
    let final_url = "https://bedrock-runtime.us-west-2.amazonaws.com/final";
    let reply = Reply::redirected(404, &[], r#"{"message":"missing"}"#, "GET", final_url);
    let error = run_direct(false, &Factory::new(reply), &data, None)
        .expect_err("the redirect target's status fails");
    let EmbeddingIoError::Provider {
        error: bedrock_error,
        ..
    } = error
    else {
        panic!("a redirect target error is a bedrock error");
    };
    assert_eq!(bedrock_error.error.response.status_code, 404);
    assert_eq!(bedrock_error.error.response.request.method, "GET");
    assert_eq!(bedrock_error.error.response.request.url, final_url);
    assert_eq!(bedrock_error.error.message, r#"{"message":"missing"}"#);
}

fn http_error() {
    provider_status_is_a_bedrock_error();
    provider_status_on_the_async_path_is_a_bedrock_error();
    status_boundaries_are_the_success_range();
    timeout_is_the_shared_handler_error();
    transport_failure_stays_a_transport_error();
    malformed_success_body_is_a_decode_error();
    non_utf8_error_body_names_the_byte_count();
    redirect_error_keeps_the_final_request();
}

fn reply_json(reply: &EmbeddingReply) -> Json {
    let text = serde_json::to_string(reply).expect("the fixture reply writes as JSON");
    serde_json::from_str(&text).expect("the fixture reply is JSON")
}

fn transform(
    model: &str,
    provider: BedrockEmbeddingProvider,
    replies: &[&str],
    batch: Option<&[&str]>,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    let parsed_replies: Vec<Object> = replies.iter().map(|text| object(text)).collect();
    let parsed_batch =
        batch.map(|entries| entries.iter().map(|text| object(text)).collect::<Vec<_>>());
    transform_response(
        &parsed_replies,
        model,
        provider,
        false,
        parsed_batch.as_deref(),
    )
}

fn transform_async(
    model: &str,
    provider: BedrockEmbeddingProvider,
    reply: &str,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    transform_response(&[object(reply)], model, provider, true, None)
}

fn assert_usage_reply(
    model: &str,
    provider: BedrockEmbeddingProvider,
    replies: &[&str],
    batch: Option<&[&str]>,
    expected: &str,
) {
    let reply = transform(model, provider, replies, batch).expect("the provider reply transforms");
    assert_eq!(reply_json(&reply), json(expected));
}

fn titan_legacy_and_v2_models() {
    let legacy = transform(
        "amazon.titan-embed-g1-text-02",
        BedrockEmbeddingProvider::Amazon,
        &[TITAN_G1_REPLY],
        None,
    )
    .expect("the legacy g1 reply transforms");
    assert_eq!(
        reply_json(&legacy),
        json(
            r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"amazon.titan-embed-g1-text-02","usage":{"prompt_tokens":7,"completion_tokens":0,"total_tokens":7,"prompt_tokens_details":null},"invocation_arn":null}"#
        )
    );
    let v2 = transform(
        "amazon.titan-embed-text-v2:0",
        BedrockEmbeddingProvider::Amazon,
        &[r#"{"embedding":[0.25],"inputTextTokenCount":3}"#],
        None,
    )
    .expect("the v2 reply transforms");
    assert_eq!(
        reply_json(&v2),
        json(
            r#"{"object":"list","data":[{"object":"embedding","embedding":[0.25],"index":0}],"model":"amazon.titan-embed-text-v2:0","usage":{"prompt_tokens":3,"completion_tokens":0,"total_tokens":3,"prompt_tokens_details":null},"invocation_arn":null}"#
        )
    );
}

fn titan_base64_embedding_is_carried_as_a_string() {
    let reply = transform(
        "amazon.titan-embed-text-v2:0",
        BedrockEmbeddingProvider::Amazon,
        &[r#"{"embedding":"AAAA","inputTextTokenCount":1}"#],
        None,
    )
    .expect("an encoded titan vector transforms");
    assert_eq!(
        reply_json(&reply),
        json(
            r#"{"object":"list","data":[{"object":"embedding","embedding":"AAAA","index":0}],"model":"amazon.titan-embed-text-v2:0","usage":{"prompt_tokens":1,"completion_tokens":0,"total_tokens":1,"prompt_tokens_details":null},"invocation_arn":null}"#
        )
    );
}

fn cohere_reply_is_unmapped() {
    let error = transform(
        COHERE_MODEL,
        BedrockEmbeddingProvider::Cohere,
        &["{}"],
        None,
    )
    .expect_err("cohere replies are not transformed here");
    assert_eq!(
        error.to_string(),
        "Unable to map model response to known provider format. model=cohere.embed-english-v3"
    );
    assert!(matches!(
        error,
        EmbeddingIoError::Unmapped { model } if model == COHERE_MODEL
    ));
}

fn titan_model_is_chosen_before_the_provider() {
    let reply = transform(
        "amazon.titan-embed-text-v1",
        BedrockEmbeddingProvider::Cohere,
        &[TITAN_G1_REPLY],
        None,
    )
    .expect("an exact titan model transforms even when the provider is cohere");
    assert_eq!(reply.model, "amazon.titan-embed-text-v1");
    assert_eq!(reply.usage.prompt_tokens, 7);
}

fn embedding_usage() {
    assert_usage_reply(
        "amazon.titan-embed-text-v1",
        BedrockEmbeddingProvider::Amazon,
        &[TITAN_G1_REPLY],
        None,
        r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"amazon.titan-embed-text-v1","usage":{"prompt_tokens":7,"completion_tokens":0,"total_tokens":7,"prompt_tokens_details":null},"invocation_arn":null}"#,
    );
    titan_legacy_and_v2_models();
    titan_base64_embedding_is_carried_as_a_string();
    assert_usage_reply(
        "amazon.titan-embed-image-v1",
        BedrockEmbeddingProvider::Amazon,
        &[r#"{"embedding":[0.5],"inputTextTokenCount":4}"#],
        Some(&[r#"{"inputImage":"aW1n"}"#, r#"{"inputText":"hello"}"#]),
        r#"{"object":"list","data":[{"object":"embedding","embedding":[0.5],"index":0}],"model":"amazon.titan-embed-image-v1","usage":{"prompt_tokens":4,"completion_tokens":0,"total_tokens":4,"prompt_tokens_details":{"image_count":1}},"invocation_arn":null}"#,
    );
    assert_usage_reply(
        MARENGO_MODEL,
        BedrockEmbeddingProvider::Twelvelabs,
        &[r#"{"data":[{"embedding":[1.5,2.5]}]}"#],
        Some(&[
            r#"{"inputType":"text"}"#,
            r#"{"inputType":"image"}"#,
            r#"{"inputType":"multi_input","multi_input":{"inputText":"hi","mediaSources":[{},{}]}}"#,
        ]),
        r#"{"object":"list","data":[{"object":"embedding","embedding":[1.5,2.5],"index":0}],"model":"twelvelabs.marengo-embed-2-7-v1:0","usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"prompt_tokens_details":{"image_count":3,"query_count":2}},"invocation_arn":null}"#,
    );
    assert_usage_reply(
        NOVA_MODEL,
        BedrockEmbeddingProvider::Nova,
        &[r#"{"embeddings":[{"embedding":[1,2,3,4],"truncatedCharLength":12}]}"#],
        Some(&[r#"{"singleEmbeddingParams":{"image":{"format":"png"}}}"#]),
        r#"{"object":"list","data":[{"object":"embedding","embedding":[1,2,3,4],"index":0}],"model":"amazon.nova-2-multimodal-embeddings-v1:0","usage":{"prompt_tokens":3,"completion_tokens":0,"total_tokens":3,"prompt_tokens_details":{"image_count":1}},"invocation_arn":null}"#,
    );
    cohere_reply_is_unmapped();
    titan_model_is_chosen_before_the_provider();
}

fn assert_placeholder(reply: &EmbeddingReply, model: &str, arn: &Json) {
    assert_eq!(reply.object, "list");
    assert_eq!(reply.model, model);
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.data[0].object, "embedding");
    assert_eq!(reply.data[0].embedding, EmbeddingVector::Floats(Vec::new()));
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.usage.prompt_tokens, 0);
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 0);
    assert!(reply.usage.prompt_tokens_details.is_none());
    assert_eq!(&reply.invocation_arn, arn);
}

fn async_placeholder_for_other_providers() {
    let reply = transform_async(
        COHERE_MODEL,
        BedrockEmbeddingProvider::Cohere,
        r#"{"invocationArn":"arn:aws:bedrock:us-west-2:1:async-invoke/abc"}"#,
    )
    .expect("the cohere async reply is a placeholder");
    assert_placeholder(
        &reply,
        COHERE_MODEL,
        &json(r#""arn:aws:bedrock:us-west-2:1:async-invoke/abc""#),
    );
}

fn async_null_arn_is_still_an_arn() {
    let reply = transform_async(
        COHERE_MODEL,
        BedrockEmbeddingProvider::Cohere,
        r#"{"invocationArn":null}"#,
    )
    .expect("a null ARN still marks the async reply");
    assert_placeholder(&reply, COHERE_MODEL, &Json::Null);
}

fn async_marengo_and_nova_replies() {
    let reply = transform_async(
        MARENGO_MODEL,
        BedrockEmbeddingProvider::Twelvelabs,
        r#"{"invocationArn":"arn:aws:bedrock:us-west-2:1:async-invoke/marengo"}"#,
    )
    .expect("the marengo async reply transforms");
    assert_placeholder(
        &reply,
        MARENGO_MODEL,
        &json(r#""arn:aws:bedrock:us-west-2:1:async-invoke/marengo""#),
    );
    let nova_reply = transform_async(
        NOVA_MODEL,
        BedrockEmbeddingProvider::Nova,
        r#"{"invocationArn":"arn:aws:bedrock:us-west-2:1:async-invoke/marengo"}"#,
    )
    .expect("the nova async reply transforms");
    assert_placeholder(
        &nova_reply,
        NOVA_MODEL,
        &json(r#""arn:aws:bedrock:us-west-2:1:async-invoke/marengo""#),
    );
}

fn arn_needs_a_single_reply_and_the_flag() {
    multiple_async_arns_take_the_regular_path();
    async_arn_requires_the_flag();
    async_arn_must_be_present();
}

fn multiple_async_arns_take_the_regular_path() {
    let replies = [
        object(r#"{"invocationArn":"arn:one"}"#),
        object(r#"{"invocationArn":"arn:two"}"#),
    ];
    let reply = transform_response(
        &replies,
        MARENGO_MODEL,
        BedrockEmbeddingProvider::Twelvelabs,
        true,
        None,
    )
    .expect("two ARN replies take the regular path");
    assert!(reply.data.is_empty());
    assert_eq!(reply.invocation_arn, Json::Null);
}

fn async_arn_requires_the_flag() {
    let single = object(r#"{"invocationArn":"arn:one"}"#);
    let error = transform_response(
        &[single],
        COHERE_MODEL,
        BedrockEmbeddingProvider::Cohere,
        false,
        None,
    )
    .expect_err("without the flag the cohere reply is unmapped");
    assert!(matches!(error, EmbeddingIoError::Unmapped { .. }));
}

fn async_arn_must_be_present() {
    let absent = object(r#"{"embedding":[0.1],"inputTextTokenCount":1}"#);
    let absent_error = transform_response(
        &[absent],
        COHERE_MODEL,
        BedrockEmbeddingProvider::Cohere,
        true,
        None,
    )
    .expect_err("an absent ARN is unmapped for cohere");
    assert!(matches!(absent_error, EmbeddingIoError::Unmapped { .. }));
    let empty_error = transform_response(
        &[],
        COHERE_MODEL,
        BedrockEmbeddingProvider::Cohere,
        true,
        None,
    )
    .expect_err("an empty reply list never enters the async branch");
    assert!(matches!(empty_error, EmbeddingIoError::Unmapped { .. }));
}

fn async_embedding_single_arn() {
    async_placeholder_for_other_providers();
    async_null_arn_is_still_an_arn();
    async_marengo_and_nova_replies();
    arn_needs_a_single_reply_and_the_flag();
}

fn load(
    parameters: &mut BTreeMap<String, Box<RawValue>>,
    environment: &BTreeMap<String, String>,
    bearer: Option<&SecretString>,
    source: &Source,
) -> Result<LoadedCredentials, EmbeddingIoError> {
    let secrets = secret_lookup(None);
    let region_worker = ManagedRegionWorker::new()?;
    runtime().block_on(load_credentials(
        CredentialRequest {
            optional_params: parameters,
            environment,
            bearer_token: bearer,
        },
        &secrets,
        &region_worker,
        source.resolve(),
    ))
}

fn explicit_region_and_popped_auth_keys() {
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN)));
    let mut parameters = raw(&[
        ("aws_region_name", "eu-central-1"),
        ("aws_access_key_id", ACCESS_KEY),
        ("aws_secret_access_key", SECRET_KEY),
        ("aws_session_token", SESSION_TOKEN),
        ("aws_session_name", "fixture-session"),
        ("temperature", "0.2"),
    ]);
    let environment = environment(&[
        ("AWS_REGION_NAME", "us-east-1"),
        ("AWS_REGION", "ap-south-1"),
    ]);
    let loaded =
        load(&mut parameters, &environment, None, &source).expect("the fixture credentials load");
    assert_eq!(loaded.region, "eu-central-1");
    let resolved = loaded
        .credentials
        .expect("the resolver supplies credentials");
    assert_credentials_and_auth_settings(&resolved, &source);
    assert_eq!(
        parameters.keys().map(String::as_str).collect::<Vec<_>>(),
        ["temperature"]
    );
}

fn assert_credentials_and_auth_settings(resolved: &Credentials, source: &Source) {
    assert_eq!(resolved.access_key_id.expose_secret(), ACCESS_KEY);
    assert_eq!(
        resolved
            .session_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(SESSION_TOKEN)
    );
    let calls = source.calls();
    assert_eq!(calls.len(), 1);
    let (settings, region) = &calls[0];
    assert_eq!(region, "eu-central-1");
    assert_eq!(settings.region_name.as_deref(), Some("eu-central-1"));
    assert_eq!(
        settings
            .access_key_id
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(ACCESS_KEY)
    );
    assert_eq!(
        settings
            .secret_access_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(SECRET_KEY)
    );
    assert_eq!(settings.session_name.as_deref(), Some("fixture-session"));
}

fn environment_region_precedence() {
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    let mut parameters = raw(&[]);
    let name_only = load(
        &mut parameters,
        &environment(&[("AWS_REGION_NAME", "us-east-1")]),
        None,
        &source,
    )
    .expect("the environment region loads");
    assert_eq!(name_only.region, "us-east-1");
    let both = load(
        &mut parameters,
        &environment(&[
            ("AWS_REGION_NAME", "us-east-1"),
            ("AWS_REGION", "ap-south-1"),
        ]),
        None,
        &source,
    )
    .expect("the standard region wins");
    assert_eq!(both.region, "ap-south-1");
    assert_eq!(source.calls().len(), 2);
}

fn boolean_like_region_leaves_the_default() {
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    let mut parameters = raw(&[]);
    let loaded = load(
        &mut parameters,
        &environment(&[("AWS_REGION_NAME", "TRUE"), ("AWS_REGION", "false")]),
        None,
        &source,
    )
    .expect("a boolean-like region is not a string");
    assert_eq!(loaded.region, "us-west-2");
}

fn empty_environment_region_counts() {
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    let mut parameters = raw(&[]);
    let loaded = load(
        &mut parameters,
        &environment(&[("AWS_REGION_NAME", "")]),
        None,
        &source,
    )
    .expect("an empty region is still a string");
    assert_eq!(loaded.region, "");
}

fn bearer_token_skips_resolution() {
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN)));
    let mut parameters = raw(&[("aws_region_name", "eu-west-1")]);
    let token = secret("");
    let loaded = load(
        &mut parameters,
        &environment(&[("AWS_REGION_NAME", "us-east-1")]),
        Some(&token),
        &source,
    )
    .expect("a bearer token skips credential resolution");
    assert!(loaded.credentials.is_none());
    assert_eq!(loaded.region, "eu-west-1");
    assert!(source.calls().is_empty());
}

fn absent_region_defaults_to_us_west_2() {
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    let mut parameters = raw(&[]);
    let loaded =
        load(&mut parameters, &environment(&[]), None, &source).expect("the default region loads");
    assert_eq!(loaded.region, "us-west-2");
}

fn embedding_credentials() {
    explicit_region_and_popped_auth_keys();
    environment_region_precedence();
    boolean_like_region_leaves_the_default();
    empty_environment_region_counts();
    bearer_token_skips_resolution();
    absent_region_defaults_to_us_west_2();
}

#[rstest]
#[case::signed_embedding_request(signed_embedding_request)]
#[case::provider_json_response(provider_json_response)]
#[case::embedding_usage(embedding_usage)]
#[case::http_error(http_error)]
#[case::embedding_credentials(embedding_credentials)]
#[case::async_embedding_single_arn(async_embedding_single_arn)]
fn bedrock_embedding_io_cases(#[case] scenario: fn()) {
    scenario();
}
