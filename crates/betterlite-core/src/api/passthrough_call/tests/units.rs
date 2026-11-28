#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::credentials::source::SourceFailure;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::super::allm_passthrough_route;
use super::super::auth;
use super::super::config::{ConfigError, PassthroughCallConfig};
use super::super::logging;
use super::super::params;
use super::support::{
    MODEL, empty_environment, failing_aws, logging_state, object, proxy_client, resolved,
    vllm_args, watsonx_environment,
};

#[test]
fn logging_json_object_maps_every_value() {
    let fields = object(&[
        ("text", JsonValue::String(String::from("x"))),
        ("number", JsonValue::from(2_i64)),
        ("flag", JsonValue::Bool(true)),
        (
            "nested",
            JsonValue::Object(object(&[("inner", JsonValue::Null)])),
        ),
    ]);
    let expected: IndexMap<String, Json> = IndexMap::from([
        (String::from("text"), Json::Text(String::from("x"))),
        (String::from("number"), Json::Number(Number::from(2_i64))),
        (String::from("flag"), Json::Bool(true)),
        (
            String::from("nested"),
            Json::Object(IndexMap::from([(String::from("inner"), Json::Null)])),
        ),
    ]);
    assert_eq!(logging::json_object(&fields), expected);
}

#[test]
fn logging_request_data_prefers_a_non_empty_data_field() {
    let mut args = vllm_args("http://127.0.0.1:1");
    let data = object(&[("query", JsonValue::String(String::from("x")))]);
    let payload = JsonValue::Object(object(&[("model", JsonValue::String(String::from("m")))]));
    args.data = Some(data.clone());
    args.json = Some(payload.clone());
    assert_eq!(
        logging::request_data(&args),
        Some(Json::Object(logging::json_object(&data)))
    );
    args.data = Some(Object::new());
    assert_eq!(logging::request_data(&args), Some(logging::json(&payload)));
    args.data = None;
    assert_eq!(logging::request_data(&args), Some(logging::json(&payload)));
    args.json = None;
    assert_eq!(logging::request_data(&args), None);
}

#[test]
fn logging_pre_call_records_the_input_the_headers_and_the_upstream_url() {
    let mut state = logging_state();
    let input = Json::Object(IndexMap::from([(
        String::from("model"),
        Json::Text(String::from("m")),
    )]));
    let mut headers = Headers::new();
    headers.insert(String::from("x-tenant"), String::from("acme"));
    let url = "https://upstream.example/v1";
    logging::record_pre_call(&mut state, &input, url, &headers).unwrap();
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("pre_api_call")))
    );
    assert_eq!(state.model_call_details.get("input"), Some(&input));
    let Some(Json::Object(additional)) = state.model_call_details.get("additional_args") else {
        panic!("the pre call records its additional args");
    };
    assert_eq!(
        additional.get("api_base"),
        Some(&Json::Text(String::from(url)))
    );
    assert_eq!(
        additional.get("headers"),
        Some(&Json::Object(IndexMap::from([(
            String::from("x-tenant"),
            Json::Text(String::from("acme")),
        )])))
    );
    assert_eq!(additional.get("complete_input_dict"), Some(&input));
    assert_eq!(
        state.litellm_params.get("api_base"),
        Some(&Json::Text(String::from(url)))
    );
}

#[test]
fn params_metadata_prefers_litellm_metadata_only_when_absent() {
    let model = resolved(MODEL, ProviderId::Vllm, None, None);
    let mut args = vllm_args("http://127.0.0.1:1");
    let fallback = object(&[("team", JsonValue::String(String::from("t")))]);
    args.extra = object(&[("litellm_metadata", JsonValue::Object(fallback.clone()))]);
    let fallback_call = params::build(&args, &model);
    assert_eq!(
        fallback_call.params.get("metadata"),
        Some(&JsonValue::Object(fallback))
    );
    args.extra = object(&[
        ("metadata", JsonValue::String(String::from("own"))),
        ("litellm_metadata", JsonValue::Object(Object::new())),
    ]);
    let explicit_call = params::build(&args, &model);
    assert_eq!(
        explicit_call.params.get("metadata"),
        Some(&JsonValue::String(String::from("own")))
    );
    args.extra = object(&[("metadata", JsonValue::Null)]);
    let null_call = params::build(&args, &model);
    assert_eq!(null_call.params.get("metadata"), Some(&JsonValue::Null));
}

#[test]
fn params_no_log_accepts_the_named_key_and_the_hyphenated_one() {
    let model = resolved(MODEL, ProviderId::Vllm, None, None);
    let mut args = vllm_args("http://127.0.0.1:1");
    args.extra = object(&[("no_log", JsonValue::Bool(true))]);
    let named_call = params::build(&args, &model);
    assert_eq!(
        named_call.params.get("no-log"),
        Some(&JsonValue::Bool(true))
    );
    args.extra = object(&[("no-log", JsonValue::String(String::from("yes")))]);
    let hyphenated_call = params::build(&args, &model);
    assert_eq!(
        hyphenated_call.params.get("no-log"),
        Some(&JsonValue::String(String::from("yes")))
    );
    args.extra = Object::new();
    let default_call = params::build(&args, &model);
    assert_eq!(default_call.params.get("no-log"), Some(&JsonValue::Null));
}

#[test]
fn params_base_model_reads_the_field_then_the_metadata() {
    let model = resolved(MODEL, ProviderId::Vllm, None, None);
    let mut args = vllm_args("http://127.0.0.1:1");
    args.extra = object(&[("base_model", JsonValue::String(String::from("b")))]);
    let field_call = params::build(&args, &model);
    assert_eq!(
        field_call.params.get("base_model"),
        Some(&JsonValue::String(String::from("b")))
    );
    let metadata = object(&[(
        "model_info",
        JsonValue::Object(object(&[(
            "base_model",
            JsonValue::String(String::from("inner")),
        )])),
    )]);
    args.extra = object(&[
        ("base_model", JsonValue::Null),
        ("metadata", JsonValue::Object(metadata)),
    ]);
    let metadata_call = params::build(&args, &model);
    assert_eq!(
        metadata_call.params.get("base_model"),
        Some(&JsonValue::String(String::from("inner")))
    );
}

#[test]
fn params_client_carries_both_timeouts() {
    let http = proxy_client(1024);
    let timeouts = params::Timeouts {
        read: Duration::from_secs(7),
        connect: Duration::from_secs(2),
    };
    let built = params::client(&http, &timeouts).unwrap();
    assert_eq!(built.limits.read_timeout, Duration::from_secs(7));
    assert_eq!(built.limits.connect_timeout, Duration::from_secs(2));
}

#[test]
fn auth_optional_text_reads_only_strings() {
    let text = object(&[("azure_ad_token", JsonValue::String(String::from("tok")))]);
    assert_eq!(
        auth::optional_text(&text, "azure_ad_token").unwrap(),
        Some(String::from("tok"))
    );
    let null = object(&[("azure_ad_token", JsonValue::Null)]);
    assert_eq!(auth::optional_text(&null, "azure_ad_token").unwrap(), None);
    assert_eq!(
        auth::optional_text(&Object::new(), "azure_ad_token").unwrap(),
        None
    );
    let flag = object(&[("azure_ad_token", JsonValue::Bool(true))]);
    let Err(error) = auth::optional_text(&flag, "azure_ad_token") else {
        panic!("a boolean is not a string");
    };
    match error {
        ConfigError::ValueError { message } => assert_eq!(
            message,
            String::from("litellm_params.azure_ad_token must be a string")
        ),
        other @ (ConfigError::Passthrough(_)
        | ConfigError::Message { .. }
        | ConfigError::Provider(_)) => panic!("unexpected error: {other}"),
    }
}

#[rstest]
#[case::azure(PassthroughConfigTag::Azure, "azure_ad_token")]
#[case::gigachat(PassthroughConfigTag::GigaChat, "gigachat_access_token")]
#[tokio::test]
async fn auth_refuses_non_string_provider_credentials(
    #[case] tag: PassthroughConfigTag,
    #[case] name: &str,
) {
    let http = proxy_client(1024);
    let params = object(&[(name, JsonValue::Bool(true))]);
    let config = PassthroughCallConfig {
        tag,
        environment: Arc::new(empty_environment()),
        aws: failing_aws(),
    };
    let Err(failure) = auth::validate(&config, None, &params, "https://example.test", &http).await
    else {
        panic!("a boolean credential is refused");
    };
    match failure {
        ConfigError::ValueError { message } => {
            assert_eq!(message, format!("litellm_params.{name} must be a string"));
        }
        other @ (ConfigError::Passthrough(_)
        | ConfigError::Message { .. }
        | ConfigError::Provider(_)) => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn auth_nvidia_nim_sets_the_bearer_header_only_for_a_key() {
    let http = proxy_client(1024);
    let key = SecretString::from("nim-key");
    let config = PassthroughCallConfig {
        tag: PassthroughConfigTag::NvidiaNim,
        environment: Arc::new(empty_environment()),
        aws: failing_aws(),
    };
    let headers = auth::validate(
        &config,
        Some(&key),
        &Object::new(),
        "https://example.test",
        &http,
    )
    .await
    .unwrap();
    assert_eq!(
        headers.get("Authorization"),
        Some(&String::from("Bearer nim-key"))
    );
    let bare = auth::validate(&config, None, &Object::new(), "https://example.test", &http)
        .await
        .unwrap();
    assert_eq!(bare.get("Authorization"), None);
}

#[tokio::test]
async fn auth_watsonx_mints_the_iam_token_from_the_key() {
    let serving = serve(
        reply(
            "200 fixture",
            &[],
            r#"{"access_token":"iam-token","expires_in":3600}"#,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let environment = watsonx_environment(&format!("http://{}/identity/token", serving.address));
    let key = SecretString::from("watsonx-key");
    let http = proxy_client(1024);
    let headers = auth::watsonx_headers(environment, Some(&key), &http)
        .await
        .unwrap();
    assert_eq!(
        headers.get("authorization"),
        Some(&String::from("Bearer iam-token"))
    );
    assert_eq!(
        headers.get("content-type"),
        Some(&String::from("application/json"))
    );
    assert_eq!(
        headers.get("accept"),
        Some(&String::from("application/json"))
    );
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /identity/token HTTP/1.1");
    assert!(sent.body.contains("apikey=watsonx-key"));
}

#[tokio::test]
async fn auth_watsonx_reports_a_refused_iam_token() {
    let serving = serve(reply("401 fixture", &[], ""), Duration::ZERO).unwrap();
    let environment = watsonx_environment(&format!("http://{}/identity/token", serving.address));
    let key = SecretString::from("watsonx-key");
    let http = proxy_client(1024);
    let Err(error) = auth::watsonx_headers(environment, Some(&key), &http).await else {
        panic!("a refused IAM token is an error");
    };
    match error {
        ConfigError::Message { status, message } => {
            assert_eq!(status, 401);
            assert_eq!(message, "the IAM token request failed with status 401");
        }
        other @ (ConfigError::ValueError { .. }
        | ConfigError::Passthrough(_)
        | ConfigError::Provider(_)) => panic!("unexpected error: {other}"),
    }
}

#[test]
fn auth_credentials_failure_keeps_its_upstream_status() {
    let refused = SourceFailure {
        class: String::from("AuthenticationError"),
        status: Some(401),
        message: String::from("token refused"),
    };
    match ConfigError::from(CredentialsError::Source(refused)) {
        ConfigError::Message { status, message } => {
            assert_eq!(status, 401);
            assert_eq!(message, "token refused");
        }
        other @ (ConfigError::Passthrough(_)
        | ConfigError::ValueError { .. }
        | ConfigError::Provider(_)) => panic!("unexpected error: {other}"),
    }
    let preflight = SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from("no token source"),
    };
    match ConfigError::from(CredentialsError::Source(preflight)) {
        ConfigError::ValueError { message } => assert_eq!(message, "no token source"),
        other @ (ConfigError::Passthrough(_)
        | ConfigError::Message { .. }
        | ConfigError::Provider(_)) => panic!("unexpected error: {other}"),
    }
}

#[test]
fn urls_vllm_without_a_base_is_a_value_error() {
    let config = PassthroughCallConfig {
        tag: PassthroughConfigTag::VLLM,
        environment: Arc::new(empty_environment()),
        aws: failing_aws(),
    };
    let params = Object::new();
    let request = PassthroughUrlRequest {
        api_base: Some(""),
        api_key: None,
        target: PassthroughTarget {
            model: MODEL,
            custom_llm_provider: "vllm",
            endpoint: "chat/completions",
        },
        request_query_params: None,
        litellm_params: &params,
    };
    let Err(error) = config.get_complete_url(&request) else {
        panic!("an empty base is refused");
    };
    match error {
        ConfigError::ValueError { message } => {
            assert!(message.contains("VLLM_API_BASE is not set"));
        }
        other @ (ConfigError::Passthrough(_)
        | ConfigError::Message { .. }
        | ConfigError::Provider(_)) => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn call_sends_the_cookies_it_is_given_and_none_for_an_empty_list() {
    let cookie_server = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut cookie_args = vllm_args(&format!("http://{}/v1", cookie_server.address));
    cookie_args.cookies = Some(vec![
        (String::from("a"), String::from("1")),
        (String::from("b"), String::from("2")),
    ]);
    let mut cookie_state = logging_state();
    let http = proxy_client(1024);
    let cookie_answer = allm_passthrough_route(
        cookie_args,
        &mut cookie_state,
        &http,
        &empty_environment(),
        &failing_aws(),
    )
    .await
    .expect("the upstream answers");
    assert_eq!(cookie_answer.status, 200);
    assert!(
        cookie_server
            .finish()
            .unwrap()
            .head
            .contains("cookie: a=1; b=2")
    );
    let empty_server = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut empty_args = vllm_args(&format!("http://{}/v1", empty_server.address));
    empty_args.cookies = Some(Vec::new());
    let mut empty_state = logging_state();
    let empty_answer = allm_passthrough_route(
        empty_args,
        &mut empty_state,
        &http,
        &empty_environment(),
        &failing_aws(),
    )
    .await
    .expect("the upstream answers");
    assert_eq!(empty_answer.status, 200);
    assert!(!empty_server.finish().unwrap().head.contains("cookie:"));
}
