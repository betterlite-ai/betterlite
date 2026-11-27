#![cfg(test)]

mod environment;
mod support;
mod units;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use bytes::Bytes;
use rstest::rstest;
use secrecy::SecretString;
use url::Url;
use url::form_urlencoded;

use self::support::{
    MODEL, bedrock_args, client_timeouts, empty_environment, header, logging_state, mantle_args,
    object, proxy_client, resolved, vllm_args,
};
use super::config::PassthroughCallConfig;
use super::params;
use super::send::arn_encoded;
use super::{PassthroughBody, PassthroughCallError, PassthroughUpload, allm_passthrough_route};

const MODEL_ARN: &str = "arn:aws:bedrock:us-west-2:123456789012:application-inference-profile/abc";
const BEDROCK_RUNTIME: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

fn full_bytes(body: PassthroughBody) -> Bytes {
    let PassthroughBody::Full(bytes) = body else {
        panic!("expected a full reply");
    };
    *bytes
}

fn json_object(pairs: &[(&str, &str)]) -> Object {
    let converted: Vec<(&str, JsonValue)> = pairs
        .iter()
        .map(|(name, value)| (*name, JsonValue::String((*value).to_owned())))
        .collect();
    object(&converted)
}

#[tokio::test]
async fn call_vllm_gets_the_url_the_headers_and_the_swapped_model() {
    let serving = serve(
        reply("200 fixture", &[("x-upstream", "yes")], "pong"),
        Duration::ZERO,
    )
    .unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.json = Some(JsonValue::Object(json_object(&[
        ("model", "vllm/original"),
        ("input", "hi"),
    ])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    assert_eq!(header(&answer, "x-upstream").as_deref(), Some("yes"));
    assert_eq!(full_bytes(answer.body).as_ref(), b"pong");
    assert_eq!(state.model, MODEL);
    assert_eq!(
        state.litellm_params.get("allm_passthrough_route"),
        Some(&Json::Bool(true))
    );
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/chat/completions HTTP/1.1");
    assert!(sent.head.contains("content-type: application/json"));
    assert!(sent.body.contains("\"model\":\"my-model\""));
    assert!(!sent.body.contains("vllm/original"));
}

#[tokio::test]
async fn call_mantle_signs_the_body_before_the_model_swap() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = mantle_args(&format!("http://{}", serving.address));
    args.api_key = Some(SecretString::from("mantle-key"));
    args.content = Some(Bytes::from_static(b"ignored"));
    args.json = Some(JsonValue::Object(json_object(&[(
        "model",
        "bedrock_mantle/original",
    )])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    assert_eq!(state.model, "original");
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer mantle-key"));
    assert!(sent.head.contains("content-type: application/json"));
    assert_eq!(sent.body, r#"{"model": "bedrock_mantle/original"}"#);
}

#[tokio::test]
async fn call_content_wins_over_data_and_json() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.content = Some(Bytes::from_static(b"raw-bytes"));
    args.data = Some(json_object(&[("query", "x")]));
    args.json = Some(JsonValue::Object(json_object(&[(
        "model",
        "vllm/original",
    )])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(full_bytes(answer.body).as_ref(), b"pong");
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("content-type: application/octet-stream"));
    assert!(!sent.head.contains("application/x-www-form-urlencoded"));
    assert_eq!(sent.body, "raw-bytes");
}

#[tokio::test]
async fn call_data_is_form_encoded_when_no_content_is_given() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    let mut data = json_object(&[("name", "ada")]);
    data.insert("count".to_owned(), JsonValue::from(3_i64));
    data.insert(
        "tag".to_owned(),
        JsonValue::Array(vec![
            JsonValue::String(String::from("a")),
            JsonValue::String(String::from("b")),
        ]),
    );
    args.data = Some(data);
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(full_bytes(answer.body).as_ref(), b"pong");
    let sent = serving.finish().unwrap();
    assert!(
        sent.head
            .contains("content-type: application/x-www-form-urlencoded")
    );
    let mut pairs: Vec<(String, String)> = form_urlencoded::parse(sent.body.as_bytes())
        .into_owned()
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            (String::from("count"), String::from("3")),
            (String::from("name"), String::from("ada")),
            (String::from("tag"), String::from("a")),
            (String::from("tag"), String::from("b")),
        ]
    );
}

#[tokio::test]
async fn call_files_build_a_multipart_body_after_the_data_fields() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.data = Some(json_object(&[("tag", "form")]));
    args.files = Some(vec![PassthroughUpload {
        field: String::from("file"),
        filename: String::from("a.txt"),
        content_type: Some(String::from("text/plain")),
        bytes: Bytes::from_static(b"hello"),
    }]);
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    let sent = serving.finish().unwrap();
    assert!(
        sent.head
            .contains("content-type: multipart/form-data; boundary=")
    );
    let form_field = sent.body.find("name=\"tag\"").expect("the data field");
    let file_field = sent.body.find("name=\"file\"").expect("the upload");
    assert!(form_field < file_field);
    assert!(sent.body.contains("filename=\"a.txt\""));
    assert!(sent.body.contains("Content-Type: text/plain"));
    assert!(sent.body.contains("hello"));
}

#[tokio::test]
async fn call_files_expand_a_list_field_into_one_part_per_item() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    let mut data = json_object(&[("name", "ada")]);
    data.insert(
        "tag".to_owned(),
        JsonValue::Array(vec![
            JsonValue::String(String::from("a")),
            JsonValue::String(String::from("b")),
        ]),
    );
    args.data = Some(data);
    args.files = Some(vec![PassthroughUpload {
        field: String::from("file"),
        filename: String::from("a.txt"),
        content_type: Some(String::from("text/plain")),
        bytes: Bytes::from_static(b"hello"),
    }]);
    let mut state = logging_state();
    let http = proxy_client(1024);
    allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert_eq!(sent.body.matches("name=\"tag\"").count(), 2);
    assert!(sent.body.contains("\r\n\r\na\r\n"));
    assert!(sent.body.contains("\r\n\r\nb\r\n"));
}

#[tokio::test]
async fn call_forwarded_headers_need_the_x_pass_prefix() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.request_headers = Some(vec![
        (String::from("x-pass-x-tenant"), String::from("acme")),
        (String::from("authorization"), String::from("Bearer caller")),
    ]);
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("x-tenant: acme"));
    assert!(!sent.head.contains("Bearer caller"));
}

#[tokio::test]
async fn call_a_redirect_is_returned_as_a_full_reply() {
    let serving = serve(reply("302 fixture", &[], ""), Duration::ZERO).unwrap();
    let args = vllm_args(&format!("http://{}/v1", serving.address));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("a redirect is an answer");
    assert_eq!(answer.status, 302);
    assert!(full_bytes(answer.body).is_empty());
    assert_eq!(
        serving.finish().unwrap().line,
        "POST /v1/chat/completions HTTP/1.1"
    );
}

#[tokio::test]
async fn call_bedrock_inference_profile_url_is_arn_encoded() {
    let profile_endpoint = format!("model/{MODEL_ARN}/converse");
    let config = PassthroughCallConfig {
        tag: PassthroughConfigTag::Bedrock,
        environment: Arc::new(empty_environment()),
    };
    let params = Object::new();
    let request = PassthroughUrlRequest {
        api_base: Some(BEDROCK_RUNTIME),
        api_key: None,
        target: PassthroughTarget {
            model: "anthropic.claude",
            custom_llm_provider: "bedrock",
            endpoint: &profile_endpoint,
        },
        request_query_params: None,
        litellm_params: &params,
    };
    let (url, base) = config.get_complete_url(&request).unwrap();
    assert_eq!(base, BEDROCK_RUNTIME);
    assert_eq!(
        url.as_str(),
        format!("{BEDROCK_RUNTIME}/{profile_endpoint}")
    );
    let bedrock = resolved("anthropic.claude", ProviderId::Bedrock, None, None);
    let encoded = arn_encoded(url, &bedrock, &profile_endpoint).unwrap();
    assert!(
        encoded
            .as_str()
            .contains(":application-inference-profile%2Fabc")
    );
}

#[tokio::test]
async fn call_arn_encoding_ignores_other_providers_and_endpoints() {
    let profile_endpoint = format!("model/{MODEL_ARN}/converse");
    let arn_url = format!("{BEDROCK_RUNTIME}/{profile_endpoint}");
    let url = Url::parse(&arn_url).unwrap();
    let vllm = resolved(MODEL, ProviderId::Vllm, None, None);
    let not_bedrock = arn_encoded(url.clone(), &vllm, &profile_endpoint).unwrap();
    assert!(!not_bedrock.as_str().contains("%2F"));
    let bedrock = resolved("anthropic.claude", ProviderId::Bedrock, None, None);
    let other_endpoint = arn_encoded(url, &bedrock, "chat/completions").unwrap();
    assert!(!other_endpoint.as_str().contains("%2F"));
}

#[tokio::test]
async fn stream_vllm_relays_the_chunks_and_sets_the_stream_flag() {
    let serving = serve(
        reply("200 fixture", &[("x-upstream", "yes")], "data: one\n\n"),
        Duration::ZERO,
    )
    .unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.json = Some(JsonValue::Object(object(&[(
        "stream",
        JsonValue::Bool(true),
    )])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the stream starts");
    assert_eq!(state.stream, Some(true));
    assert_eq!(answer.status, 200);
    assert_eq!(header(&answer, "x-upstream").as_deref(), Some("yes"));
    let PassthroughBody::Stream(mut relay) = answer.body else {
        panic!("expected a stream");
    };
    let mut body = Vec::new();
    while let Some(chunk) = relay.next_chunk().await.expect("the relay reads") {
        body.extend_from_slice(&chunk);
    }
    assert_eq!(body, b"data: one\n\n".to_vec());
    relay.background_tasks.close();
    relay.background_tasks.wait().await;
    assert!(
        serving
            .finish()
            .unwrap()
            .line
            .starts_with("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn stream_without_the_key_is_not_streaming() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.json = Some(JsonValue::Object(json_object(&[("input", "hi")])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("the upstream answers");
    assert_eq!(state.stream, Some(false));
    assert_eq!(full_bytes(answer.body).as_ref(), b"pong");
    assert!(
        serving
            .finish()
            .unwrap()
            .line
            .starts_with("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn stream_redirect_answer_becomes_a_relay() {
    let serving = serve(reply("307 fixture", &[], "moved"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.json = Some(JsonValue::Object(object(&[(
        "stream",
        JsonValue::Bool(true),
    )])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let answer = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .expect("a redirect still streams");
    assert_eq!(answer.status, 307);
    let PassthroughBody::Stream(mut relay) = answer.body else {
        panic!("expected a stream");
    };
    assert_eq!(relay.status_code().unwrap(), 307);
    assert_eq!(relay.next_chunk().await.unwrap(), Some(b"moved".to_vec()));
    assert_eq!(relay.next_chunk().await.unwrap(), None);
    relay.background_tasks.close();
    relay.background_tasks.wait().await;
}

#[test]
fn params_carry_the_flag_the_key_and_the_model_id() {
    let model = resolved(
        "anthropic.claude",
        ProviderId::Bedrock,
        Some(BEDROCK_RUNTIME),
        Some("bedrock-key"),
    );
    let mut args = bedrock_args(BEDROCK_RUNTIME);
    args.extra = object(&[
        ("allm_passthrough_route", JsonValue::Bool(true)),
        ("model_id", JsonValue::String(String::from(MODEL_ARN))),
        (
            "custom_llm_provider",
            JsonValue::String(String::from("ignored")),
        ),
    ]);
    let call = params::build(&args, &model);
    assert_eq!(
        call.params.get("allm_passthrough_route"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        call.params.get("model_id"),
        Some(&JsonValue::String(String::from(MODEL_ARN)))
    );
    assert_eq!(
        call.params.get("api_base"),
        Some(&JsonValue::String(String::from(BEDROCK_RUNTIME)))
    );
    assert_eq!(
        call.params.get("api_key"),
        Some(&JsonValue::String(String::from("bedrock-key")))
    );
    assert_eq!(
        call.params.get("custom_llm_provider"),
        Some(&JsonValue::Null)
    );
    assert!(call.logged.contains_key("model_id"));
}

#[test]
fn params_pass_an_extra_timeout_key_through() {
    let model = resolved(MODEL, ProviderId::Vllm, None, None);
    let mut args = vllm_args("http://127.0.0.1:1");
    args.extra = object(&[("force_timeout", JsonValue::from(300_i64))]);
    let call = params::build(&args, &model);
    assert_eq!(
        call.params.get("force_timeout"),
        Some(&JsonValue::from(300_i64))
    );
}

#[test]
fn params_a_given_client_timeout_wins() {
    let model = resolved(MODEL, ProviderId::Vllm, None, None);
    let mut args = vllm_args("http://127.0.0.1:1");
    args.client = Some(client_timeouts());
    args.extra = object(&[("timeout", JsonValue::from(99_i64))]);
    let call = params::build(&args, &model);
    let resolved_timeouts = params::timeouts(&args, &call.params).unwrap();
    assert_eq!(resolved_timeouts.read, Duration::from_secs(30));
    assert_eq!(resolved_timeouts.connect, Duration::from_secs(3));
}

#[rstest]
#[case(vec![], 600)]
#[case(vec![("timeout", JsonValue::from(42_i64))], 42)]
#[case(
    vec![("stream", JsonValue::Bool(true)), ("stream_timeout", JsonValue::from(9_i64))],
    9
)]
#[test]
fn params_without_a_client_resolve_the_timeout(
    #[case] pairs: Vec<(&str, JsonValue)>,
    #[case] seconds: u64,
) {
    let model = resolved(MODEL, ProviderId::Vllm, None, None);
    let mut args = vllm_args("http://127.0.0.1:1");
    args.extra = object(&pairs);
    let call = params::build(&args, &model);
    let resolved_timeouts = params::timeouts(&args, &call.params).unwrap();
    assert_eq!(resolved_timeouts.read, Duration::from_secs(seconds));
    assert_eq!(resolved_timeouts.connect, Duration::from_secs(seconds));
}

#[tokio::test]
async fn error_upstream_4xx_is_reported_with_its_body() {
    let serving = serve(
        reply("429 fixture", &[("x-upstream", "yes")], "slow down"),
        Duration::ZERO,
    )
    .unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.json = Some(JsonValue::Object(json_object(&[("input", "hi")])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let error = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .err()
        .expect("429 is an error");
    let PassthroughCallError::UpstreamStatus {
        status,
        headers,
        body,
    } = error
    else {
        panic!("expected the upstream status");
    };
    assert_eq!(status, 429);
    assert_eq!(body.as_ref(), b"slow down");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "x-upstream" && value == "yes")
    );
    assert!(
        serving
            .finish()
            .unwrap()
            .line
            .starts_with("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn error_a_streaming_5xx_is_reported_before_the_stream() {
    let serving = serve(reply("503 fixture", &[], "boom"), Duration::ZERO).unwrap();
    let mut args = vllm_args(&format!("http://{}/v1", serving.address));
    args.json = Some(JsonValue::Object(object(&[(
        "stream",
        JsonValue::Bool(true),
    )])));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let error = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .err()
        .expect("503 is an error");
    let PassthroughCallError::UpstreamStatus { status, body, .. } = error else {
        panic!("expected the upstream status");
    };
    assert_eq!(status, 503);
    assert_eq!(body.as_ref(), b"boom");
    assert!(
        serving
            .finish()
            .unwrap()
            .line
            .starts_with("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn error_a_provider_without_a_passthrough_config_is_named() {
    let mut args = vllm_args("http://127.0.0.1:1");
    args.model = String::from("gpt-4o");
    args.custom_llm_provider = Some(String::from("openai"));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let error = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .err()
        .expect("openai has no passthrough config");
    assert!(
        matches!(&error, PassthroughCallError::ProviderNotFound(name) if name.as_str() == "openai")
    );
    assert_eq!(error.to_string(), "Provider openai not found");
}

#[tokio::test]
async fn error_a_model_without_a_provider_is_a_resolution_error() {
    let mut args = vllm_args("http://127.0.0.1:1");
    args.model = String::from("no-such-model-anywhere");
    args.custom_llm_provider = None;
    args.api_base = None;
    let mut state = logging_state();
    let http = proxy_client(1024);
    let error = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .err()
        .expect("no provider can be found for the model");
    assert!(matches!(error, PassthroughCallError::Resolution(_)));
}

#[tokio::test]
async fn error_a_send_failure_goes_through_handle_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let args = vllm_args(&format!("http://{address}/v1"));
    let mut state = logging_state();
    let http = proxy_client(1024);
    let error = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .err()
        .expect("the connection is refused");
    let PassthroughCallError::Mapped(provider) = &error else {
        panic!("expected the provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert!(!provider.message.is_empty());
}

#[tokio::test]
async fn error_bedrock_without_credentials_is_a_mapped_value_error() {
    let args = bedrock_args(BEDROCK_RUNTIME);
    let mut state = logging_state();
    let http = proxy_client(1024);
    let error = allm_passthrough_route(args, &mut state, &http, &empty_environment())
        .await
        .err()
        .expect("signing has no credentials");
    let PassthroughCallError::Mapped(provider) = &error else {
        panic!("expected the provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "Unable to locate credentials");
}
