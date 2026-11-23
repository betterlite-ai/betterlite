#![cfg(test)]

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::str::from_utf8;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::config::AnthropicSkillsConfig;
use betterlite_anthropic::skills::wire::CreateSkillRequest;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::skills::create::handler::CreateSkillHttpRequest;
use betterlite_core::api::skills::get_delete::handler::{
    SkillHttpRequest, SkillsHandlerError, SkillsHttpHandler,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use pyo3_async_runtimes::tokio::get_runtime;

use super::{Exchange, HttpSkills, create_body, ensure_success, transport_failure};

fn config() -> AnthropicSkillsConfig {
    AnthropicSkillsConfig {
        settings: AnthropicSettings::default(),
    }
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn create_request<'request>(
    body: &'request CreateSkillRequest,
    config: &'request AnthropicSkillsConfig,
) -> CreateSkillHttpRequest<'request> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    CreateSkillHttpRequest {
        url: "https://example.test/v1/skills".to_owned(),
        headers,
        timeout: Duration::from_secs(5),
        body,
        config,
        custom_llm_provider: "anthropic",
    }
}

fn provider(error: SkillsHandlerError) -> (u16, String, bool) {
    let SkillsHandlerError::Provider {
        status_code,
        message,
        status_code_is_synthesized,
        ..
    } = error
    else {
        panic!("a provider failure is what the transport reports");
    };
    (status_code, message, status_code_is_synthesized)
}

// A port nothing listens on any more: the connect is refused at once instead of waiting.
fn unused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[test]
fn create_without_files_sends_json() {
    let request = CreateSkillRequest {
        display_title: Some("T".to_owned()),
        files: None,
        extra: IndexMap::new(),
    };
    let settings = config();
    let http_request = create_request(&request, &settings);
    let blobs = Mutex::new(Vec::new());
    let (headers, body) = create_body(&http_request, &blobs).unwrap();
    let RequestBody::Json(encoded) = body else {
        panic!("a create without files is a JSON body");
    };
    assert_eq!(
        String::from_utf8(encoded).unwrap(),
        r#"{"display_title":"T"}"#
    );
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

#[test]
fn create_with_an_empty_file_list_sends_json() {
    let request = CreateSkillRequest {
        display_title: Some("T".to_owned()),
        files: Some(Vec::new()),
        extra: IndexMap::new(),
    };
    let settings = config();
    let http_request = create_request(&request, &settings);
    let blobs = Mutex::new(Vec::new());
    let (_, body) = create_body(&http_request, &blobs).unwrap();
    let RequestBody::Json(encoded) = body else {
        panic!("an empty file list is still a JSON body");
    };
    assert_eq!(
        String::from_utf8(encoded).unwrap(),
        r#"{"display_title":"T","files":[]}"#
    );
}

#[test]
fn create_with_files_sends_multipart_parts_in_order() {
    let request: CreateSkillRequest = serde_json::from_str(
        r#"{"display_title":"T","description":"D","flag":true,"count":3,"nothing":null,"nested":{"k":1},"list":[1],"files":[{"$file":{"name":"a.pdf","blob":0}},{"$file":{"name":"noext","blob":1}}]}"#,
    )
    .unwrap();
    let settings = config();
    let http_request = create_request(&request, &settings);
    let blobs = Mutex::new(vec![b"one".to_vec(), b"two".to_vec()]);
    let (headers, body) = create_body(&http_request, &blobs).unwrap();
    let RequestBody::Multipart { parts, .. } = body else {
        panic!("files make a multipart body");
    };
    assert!(headers.get(CONTENT_TYPE).is_none());
    let names: Vec<&str> = parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "display_title",
            "description",
            "flag",
            "count",
            "nothing",
            "nested",
            "list",
            "files[]",
            "files[]",
        ]
    );
    let texts: Vec<&str> = parts
        .iter()
        .take(7)
        .map(|part| from_utf8(&part.data).unwrap())
        .collect();
    assert_eq!(texts, ["T", "D", "true", "3", "", r#"{"k":1}"#, "[1]"]);
    assert_eq!(parts[7].file_name.as_deref(), Some("a.pdf"));
    assert_eq!(parts[7].content_type.as_deref(), Some("application/pdf"));
    assert_eq!(parts[7].data, b"one");
    assert_eq!(parts[8].file_name.as_deref(), Some("noext"));
    assert_eq!(
        parts[8].content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(parts[8].data, b"two");
}

#[test]
fn a_success_status_is_ok() {
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: Vec::new(),
    };
    ensure_success(&response, &config()).unwrap();
}

#[test]
fn an_error_status_is_the_providers_error() {
    let response = UpstreamResponse {
        status: 404,
        headers: HeaderMap::new(),
        body: br#"{"error":"missing"}"#.to_vec(),
    };
    let error = ensure_success(&response, &config()).err().unwrap();
    let (status_code, message, synthesized) = provider(error);
    assert_eq!(status_code, 404);
    assert!(!synthesized);
    assert!(message.contains("missing"), "{message}");
}

#[test]
fn a_non_timeout_error_is_a_server_error_whatever_the_handler() {
    let error = transport_failure(
        &TransportError::HeadersTooLarge { limit: 4 },
        Exchange {
            after: Duration::from_secs(30),
            timeout_status: Some(408_u16),
        },
    );
    let (status_code, _, synthesized) = provider(error);
    assert_eq!(status_code, 500);
    assert!(synthesized);
}

#[test]
fn an_error_without_a_timeout_status_is_a_server_error() {
    let error = transport_failure(
        &TransportError::Closed,
        Exchange {
            after: Duration::from_secs(30),
            timeout_status: None,
        },
    );
    let (status_code, _, synthesized) = provider(error);
    assert_eq!(status_code, 500);
    assert!(synthesized);
}

#[test]
fn a_refused_connection_is_a_server_error() {
    let port = unused_port();
    let settings = config();
    let outcome = get_runtime().block_on(async {
        let http = HttpSkills { client: client() };
        http.delete_skill(SkillHttpRequest {
            url: format!("http://127.0.0.1:{port}/v1/skills/s"),
            headers: HeaderMap::new(),
            timeout: Duration::from_millis(100),
            config: &settings,
            custom_llm_provider: "anthropic",
        })
        .await
    });
    let (status_code, message, synthesized) = provider(outcome.err().unwrap());
    assert_eq!(status_code, 500);
    assert!(synthesized);
    assert!(
        message.contains("the connection could not be made"),
        "{message}"
    );
}

#[test]
fn a_delete_deadline_is_the_handlers_timeout_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let settings = config();
    let outcome = get_runtime().block_on(async {
        let http = HttpSkills { client: client() };
        http.delete_skill(SkillHttpRequest {
            url: format!("http://127.0.0.1:{port}/v1/skills/s"),
            headers: HeaderMap::new(),
            timeout: Duration::from_millis(100),
            config: &settings,
            custom_llm_provider: "anthropic",
        })
        .await
    });
    drop(listener);
    let (status_code, message, synthesized) = provider(outcome.err().unwrap());
    assert_eq!(status_code, 408);
    assert!(!synthesized);
    assert_eq!(message, "Connection timed out after 0.1 seconds.");
}
