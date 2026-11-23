#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::params::ParameterError;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::support;
use crate::openai_dispatch::OpenAi;
use crate::openai_dispatch::fixtures::answer::RecordedRequest;
use crate::operation::{ChatOperation, OperationError, OperationRequest};
use crate::prepared::deployment::Deployment;
use crate::provider::Adapter as _;

const MODEL: &str = "gpt-4";
const MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;
// Long enough that a refused call would have dialled the loopback had it tried.
const QUIET: Duration = Duration::from_millis(100);

fn chat(body: &str) -> OperationRequest {
    OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from(MODEL),
        request: support::chat_request(body),
        control: ControlOptions::default(),
    }))
}

fn policy(drop_params: Option<bool>, additional: &[&str]) -> DeploymentDropPolicy {
    DeploymentDropPolicy {
        drop_params,
        additional_drop_params: additional.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn deployment(api_base: &str, drop_policy: DeploymentDropPolicy) -> Deployment {
    let mut chat = ChatDeployment::resolve(
        &ModelDeployment {
            model_name: String::from("gpt-4-group"),
            litellm_params: LiteLlmParams {
                ssl_verify: None,
                model: Some(format!("openai/{MODEL}")),
                api_base: Some(String::from(api_base)),
                api_key: Some(SecretString::from("test-key")),
                api_version: None,
                timeout: None,
                stream_timeout: None,
                max_retries: None,
                rpm: None,
                tpm: None,
                weight: None,
            },
            model_info: ModelInfo::default(),
        },
        &OpenAiDefaults::default(),
    )
    .expect("the deployment resolves");
    chat.drop_policy = drop_policy;
    Deployment::from_chat(&chat)
}

async fn loopback() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    (listener, format!("http://{address}/v1"))
}

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    ProxiedClient::outbound(&policy).expect("the client")
}

fn reply() -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        support::CHAT_REPLY.len(),
        support::CHAT_REPLY
    )
}

async fn serve_once(listener: TcpListener, expected: &[u8]) -> Vec<u8> {
    let (mut stream, _peer) = listener.accept().await.expect("the connection");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(expected) {
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the body");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
    stream
        .write_all(reply().as_bytes())
        .await
        .expect("the reply");
    received
}

async fn sent_body(
    drop_policy: DeploymentDropPolicy,
    request: &str,
) -> IndexMap<String, JsonValue> {
    let (listener, api_base) = loopback().await;
    let deployment = deployment(&api_base, drop_policy);
    let prepared = OpenAi
        .prepare(&chat(request), &deployment, &support::composition().catalog)
        .expect("the deployment prepares chat");
    let expected = RecordedRequest::of(&prepared)
        .expect("a prepared chat request")
        .body;
    let (result, received) = tokio::join!(
        async {
            let client = client();
            OpenAi.execute(&client, &prepared, &deployment).await
        },
        serve_once(listener, &expected),
    );
    result.expect("the loopback answers the call");
    let start = received
        .len()
        .checked_sub(expected.len())
        .expect("the body arrived with the request");
    support::body_fields(&received[start..])
}

async fn refused(drop_policy: DeploymentDropPolicy, request: &str) -> OperationError {
    let (listener, api_base) = loopback().await;
    let deployment = deployment(&api_base, drop_policy);
    let error = OpenAi
        .prepare(&chat(request), &deployment, &support::composition().catalog)
        .expect_err("the parameter check refuses the call");
    assert!(
        timeout(QUIET, listener.accept()).await.is_err(),
        "a refused call sends no request"
    );
    error
}

fn unsupported_request() -> String {
    format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"response_format":{{"type":"json_object"}}}}"#
    )
}

fn temperature_request() -> String {
    format!(r#"{{"model":"{MODEL}","messages":{MESSAGES},"temperature":0.5,"top_p":0.9}}"#)
}

pub(super) async fn deployment_additional_drop_params() {
    let body = sent_body(policy(None, &["temperature"]), &temperature_request()).await;
    assert_eq!(
        support::field(&body, "top_p"),
        &support::real_value(0.9),
        "the deployment list leaves the other parameter alone"
    );
    assert!(
        body.get("temperature").is_none(),
        "the deployment additional_drop_params takes temperature out"
    );
    assert!(body.get("additional_drop_params").is_none());
    request_list_replaces_the_deployment_list().await;
    provider_name_dropped().await;
    standard_parameter_nested_paths().await;
    nested_path_dropped().await;
}

async fn request_list_replaces_the_deployment_list() {
    let replaced = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"temperature":0.5,"top_p":0.9,"additional_drop_params":["top_p"]}}"#
    );
    let body = sent_body(policy(None, &["temperature"]), &replaced).await;
    assert_eq!(
        support::field(&body, "temperature"),
        &support::real_value(0.5),
        "the request list replaces the deployment's, so temperature stays"
    );
    assert!(
        body.get("top_p").is_none(),
        "the request additional_drop_params takes top_p out"
    );
}

async fn provider_name_dropped() {
    let provider = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"x_custom":1,"additional_drop_params":["x_custom"]}}"#
    );
    let body = sent_body(DeploymentDropPolicy::default(), &provider).await;
    assert!(
        body.get("x_custom").is_none(),
        "a provider-specific name the list names is not sent either"
    );
    // a dropped name leaves the request's own extra_body, and its other keys reach the body at the top level
    let in_extra_body = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"extra_body":{{"x_secret":1,"keep":2}},"additional_drop_params":["x_secret"]}}"#
    );
    let base_body = sent_body(DeploymentDropPolicy::default(), &in_extra_body).await;
    assert!(
        base_body.get("x_secret").is_none(),
        "the drop list filters the extra_body merge"
    );
    assert_eq!(
        support::field(&base_body, "keep"),
        &support::number_value(2),
        "the other extra_body key is sent at the top level"
    );
}

async fn standard_parameter_nested_paths() {
    // utils.py L5035-L5041: a dotted additional_drop_params entry deletes the nested key and the call goes out
    let nested = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"logit_bias":{{"1":2,"2":3}},"additional_drop_params":["logit_bias.1"]}}"#
    );
    let body = sent_body(DeploymentDropPolicy::default(), &nested).await;
    let expected = IndexMap::from([(String::from("2"), support::number_value(3))]);
    assert_eq!(
        support::field(&body, "logit_bias"),
        &JsonValue::Object(expected),
        "the nested path removes only the named key"
    );
    let metadata = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"metadata":{{"trace_id":"t-1"}},"additional_drop_params":["metadata.trace_id"]}}"#
    );
    let served = sent_body(DeploymentDropPolicy::default(), &metadata).await;
    assert!(
        served.get("metadata").is_none(),
        "a nested path under metadata still serves the call"
    );
}

async fn nested_path_dropped() {
    // LiteLLM moves a provider-specific param under extra_body before the delete, so a bare name finds nothing
    let forwarded = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"x_custom":{{"keep":1,"drop":2}},"additional_drop_params":["x_custom.drop"]}}"#
    );
    let untouched = sent_body(DeploymentDropPolicy::default(), &forwarded).await;
    let both = IndexMap::from([
        (String::from("keep"), support::number_value(1)),
        (String::from("drop"), support::number_value(2)),
    ]);
    assert_eq!(
        support::field(&untouched, "x_custom"),
        &JsonValue::Object(both),
        "a provider-specific param sits under extra_body, so the bare name deletes nothing"
    );
    let under_extra_body = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"x_custom":{{"keep":1,"drop":2}},"additional_drop_params":["extra_body.x_custom.drop"]}}"#
    );
    let reduced = sent_body(DeploymentDropPolicy::default(), &under_extra_body).await;
    let kept = IndexMap::from([(String::from("keep"), support::number_value(1))]);
    assert_eq!(
        support::field(&reduced, "x_custom"),
        &JsonValue::Object(kept),
        "the extra_body path reaches the provider-specific param"
    );
}

pub(super) async fn openai_unsupported_param() {
    let error = refused(DeploymentDropPolicy::default(), &unsupported_request()).await;
    let OperationError::Parameters {
        source: ParameterError::Unsupported { names, .. },
    } = error
    else {
        panic!("an unsupported parameter is a Parameters error");
    };
    assert_eq!(names, vec![String::from("response_format")]);
}

pub(super) async fn openai_unsupported_param_dropped() {
    let body = sent_body(policy(Some(true), &[]), &unsupported_request()).await;
    assert!(
        body.get("response_format").is_none(),
        "the deployment flag drops the unsupported parameter"
    );
    let requested = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"response_format":{{"type":"json_object"}},"drop_params":true}}"#
    );
    let requested_body = sent_body(DeploymentDropPolicy::default(), &requested).await;
    assert!(
        requested_body.get("response_format").is_none(),
        "the request flag drops the unsupported parameter"
    );
    assert!(requested_body.get("drop_params").is_none());
}

pub(super) async fn openai_allowed_param() {
    let allowed = format!(
        r#"{{"model":"{MODEL}","messages":{MESSAGES},"response_format":{{"type":"json_object"}},"allowed_openai_params":["response_format"]}}"#
    );
    let body = sent_body(DeploymentDropPolicy::default(), &allowed).await;
    let object = IndexMap::from([(
        String::from("type"),
        JsonValue::Text(String::from("json_object")),
    )]);
    assert_eq!(
        support::field(&body, "response_format"),
        &JsonValue::Object(object),
        "the allowed parameter reaches the body as sent"
    );
    assert!(body.get("allowed_openai_params").is_none());
}
