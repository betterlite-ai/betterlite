use std::collections::BTreeMap;
use std::future::Future;

use betterlite_prompts::arguments::json::Json;
use http::Method;
use rstest::rstest;

use crate::credential_contracts::errors::CredError;
use crate::credentials::McpAuthType;
use crate::openapi_tools::request_headers::OpenApiCallContext;
use crate::openapi_tools::tool::{OpenApiTool, OpenApiToolError, OpenApiToolOutput};
use crate::openapi_tools::transport::OpenApiRequest;

use super::support::{BASE, FakeTransport, arguments, config, context, json, tool};

const BODY: &str = r#"{"requestBody":{"content":{"application/json":{}}}}"#;

fn text(value: &str) -> OpenApiToolOutput {
    OpenApiToolOutput::Text(value.to_owned())
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn body_json(request: &OpenApiRequest) -> Json {
    serde_json::from_slice(request.body.as_deref().unwrap()).unwrap()
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn owner_tool() -> OpenApiTool {
    tool(
        "get",
        "/repos/{owner}",
        r#"{"parameters":[{"name":"owner","in":"path"}]}"#,
    )
}

fn item_tool() -> OpenApiTool {
    tool(
        "get",
        "/items/{id}",
        r#"{"parameters":[{"name":"id","in":"path"}]}"#,
    )
}

async fn exchange(
    tool: &OpenApiTool,
    args: &str,
    context: &OpenApiCallContext,
) -> (OpenApiRequest, OpenApiToolOutput) {
    let transport = FakeTransport::replying(200, b"{}");
    let output = tool
        .call(&arguments(args), context, &transport)
        .await
        .unwrap();
    (transport.only_request(), output)
}

async fn sent(tool: &OpenApiTool, args: &str) -> (OpenApiRequest, OpenApiToolOutput) {
    exchange(tool, args, &context()).await
}

async fn sent_with(tool: &OpenApiTool, args: &str, context: &OpenApiCallContext) -> OpenApiRequest {
    let (request, output) = exchange(tool, args, context).await;
    assert_eq!(
        output,
        OpenApiToolOutput::Json {
            value: json("{}"),
            original_text: "{}".to_owned()
        }
    );
    request
}

async fn failed_call(tool: &OpenApiTool, args: &str) -> OpenApiToolError {
    let transport = FakeTransport::replying(200, b"{}");
    let call_context = context();
    let error = tool
        .call(&arguments(args), &call_context, &transport)
        .await
        .unwrap_err();
    assert!(transport.requests().is_empty());
    error
}

#[test]
fn builds_the_url_from_the_base_path_and_path_arguments() {
    let tool = tool(
        "get",
        "/repos/{owner}/{repo}",
        r#"{"parameters":[{"name":"owner","in":"path"},{"name":"repo","in":"path"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"owner":"o","repo":"r"}"#));
    assert_eq!(request.url, format!("{BASE}/repos/o/r"));
    assert_eq!(request.method, Method::GET);
    assert!(request.query.is_empty());
    assert_eq!(request.body, None);
}

#[test]
fn a_missing_path_argument_leaves_its_placeholder() {
    let tool = tool(
        "get",
        "/repos/{owner}/{repo}",
        r#"{"parameters":[{"name":"owner","in":"path"},{"name":"repo","in":"path"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"owner":"o"}"#));
    assert_eq!(request.url, format!("{BASE}/repos/o/{{repo}}"));
}

#[rstest]
#[case::empty_string(r#"{"owner":""}"#)]
#[case::zero(r#"{"owner":0}"#)]
#[case::bool_false(r#"{"owner":false}"#)]
#[case::null(r#"{"owner":null}"#)]
#[case::empty_list(r#"{"owner":[]}"#)]
#[case::empty_object(r#"{"owner":{}}"#)]
fn a_falsy_path_argument_leaves_its_placeholder(#[case] args: &str) {
    let (request, _) = block_on(sent(&owner_tool(), args));
    assert_eq!(request.url, format!("{BASE}/repos/{{owner}}"));
}

#[rstest]
#[case::forward_slash(
    r#"{"owner":"a/b"}"#,
    "Path parameter 'owner' must not contain path separators"
)]
#[case::backslash(
    r#"{"owner":"a\\b"}"#,
    "Path parameter 'owner' must not contain path separators"
)]
#[case::dot_dot(
    r#"{"owner":".."}"#,
    "Path parameter 'owner' cannot include '.' or '..' segments"
)]
fn a_forbidden_path_value_is_a_text_result(#[case] args: &str, #[case] message: &str) {
    let transport = FakeTransport::replying(200, b"{}");
    let tool = owner_tool();
    let output = block_on(tool.call(&arguments(args), &context(), &transport)).unwrap();
    let expected = format!("Invalid path parameter: {message}");
    assert_eq!(output, text(&expected));
    assert!(transport.requests().is_empty());
}

#[test]
fn a_single_dot_path_value_is_encoded_as_a_dot() {
    let (request, _) = block_on(sent(&owner_tool(), r#"{"owner":"."}"#));
    assert_eq!(request.url, format!("{BASE}/repos/."));
}

#[test]
fn path_values_are_percent_encoded() {
    let tool = tool(
        "get",
        "/items/{name}",
        r#"{"parameters":[{"name":"name","in":"path"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"name":"a b?c&d=e"}"#));
    assert_eq!(request.url, format!("{BASE}/items/a%20b%3Fc%26d%3De"));
}

#[rstest]
#[case::integer(r#"{"id":42}"#, "/items/42")]
#[case::bool_true(r#"{"id":true}"#, "/items/True")]
#[case::decimal(r#"{"id":1.5}"#, "/items/1.5")]
#[case::non_ascii(r#"{"id":"ü"}"#, "/items/%C3%BC")]
fn a_path_value_is_taken_as_text(#[case] args: &str, #[case] expected: &str) {
    let (request, _) = block_on(sent(&item_tool(), args));
    assert_eq!(request.url, format!("{BASE}{expected}"));
}

#[test]
fn the_single_brace_replacement_runs_before_the_double() {
    let tool = tool(
        "get",
        "/x/{{id}}",
        r#"{"parameters":[{"name":"id","in":"path"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"id":"v"}"#));
    assert_eq!(request.url, format!("{BASE}/x/{{v}}"));
}

#[test]
fn every_placeholder_in_the_path_is_replaced() {
    let tool = tool(
        "get",
        "/{id}/x/{id}",
        r#"{"parameters":[{"name":"id","in":"path"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"id":"v"}"#));
    assert_eq!(request.url, format!("{BASE}/v/x/v"));
}

#[test]
fn query_arguments_follow_the_parameter_order() {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"b","in":"query"},{"name":"a","in":"query"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"a":1,"b":2}"#));
    assert_eq!(
        request.query,
        vec![
            ("b".to_owned(), "2".to_owned()),
            ("a".to_owned(), "1".to_owned())
        ]
    );
}

#[rstest]
#[case::empty_string(r#"{"q":""}"#)]
#[case::zero(r#"{"q":0}"#)]
#[case::bool_false(r#"{"q":false}"#)]
#[case::null(r#"{"q":null}"#)]
#[case::empty_list(r#"{"q":[]}"#)]
#[case::empty_object(r#"{"q":{}}"#)]
fn a_falsy_query_argument_is_not_sent(#[case] args: &str) {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"q","in":"query"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, args));
    assert!(request.query.is_empty());
}

#[test]
fn a_list_query_argument_is_repeated() {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"ids","in":"query"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"ids":[1,true,"x"]}"#));
    assert_eq!(
        request.query,
        vec![
            ("ids".to_owned(), "1".to_owned()),
            ("ids".to_owned(), "true".to_owned()),
            ("ids".to_owned(), "x".to_owned()),
        ]
    );
}

#[test]
fn scalar_query_values_use_wire_text() {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"flag","in":"query"},{"name":"limit","in":"query"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"flag":true,"limit":2.5}"#));
    assert_eq!(
        request.query,
        vec![
            ("flag".to_owned(), "true".to_owned()),
            ("limit".to_owned(), "2.5".to_owned()),
        ]
    );
}

#[test]
fn an_object_query_argument_goes_out_as_compact_json() {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"filter","in":"query"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"filter":{"a":1}}"#));
    assert_eq!(
        request.query,
        vec![("filter".to_owned(), r#"{"a":1}"#.to_owned())]
    );
}

#[test]
fn an_argument_that_is_not_a_parameter_is_not_sent() {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"q","in":"query"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"q":"x","extra":"y","id":5}"#));
    assert_eq!(request.query, vec![("q".to_owned(), "x".to_owned())]);
}

#[test]
fn a_header_parameter_is_never_sent() {
    let tool = tool(
        "get",
        "/pets",
        r#"{"parameters":[{"name":"X-Trace","in":"header"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"X-Trace":"t"}"#));
    assert!(request.headers.is_empty());
}

#[test]
fn a_post_sends_the_body_object_as_json() {
    let tool = tool("post", "/pets", BODY);
    let (request, _) = block_on(sent(&tool, r#"{"body":{"name":"rex"}}"#));
    assert_eq!(request.method, Method::POST);
    assert_eq!(body_json(&request), json(r#"{"name":"rex"}"#));
}

#[test]
fn a_missing_body_argument_with_a_request_body_sends_an_empty_object() {
    let tool = tool("post", "/pets", BODY);
    let (request, _) = block_on(sent(&tool, "{}"));
    assert_eq!(body_json(&request), json("{}"));
}

#[test]
fn an_empty_object_body_is_sent() {
    let tool = tool("post", "/pets", BODY);
    let (request, _) = block_on(sent(&tool, r#"{"body":{}}"#));
    assert_eq!(body_json(&request), json("{}"));
}

#[rstest]
#[case::array_text(r#"{"body":"[1,2]"}"#, "[1,2]")]
#[case::number_text(r#"{"body":"5"}"#, "5")]
#[case::nested_object(r#"{"body":"{\"a\":1}"}"#, r#"{"a":1}"#)]
#[case::bare_number(r#"{"body":7}"#, r#"{"data":7}"#)]
#[case::bool_true(r#"{"body":true}"#, r#"{"data":true}"#)]
#[case::array_value(r#"{"body":[1]}"#, r#"{"data":[1]}"#)]
fn a_body_argument_is_parsed_or_wrapped_as_data(#[case] args: &str, #[case] expected: &str) {
    let (request, _) = block_on(sent(&tool("post", "/pets", BODY), args));
    assert_eq!(body_json(&request), json(expected));
}

#[test]
fn a_null_string_body_sends_no_body() {
    let tool = tool("post", "/pets", BODY);
    let (request, _) = block_on(sent(&tool, r#"{"body":"null"}"#));
    assert_eq!(request.body, None);
}

#[test]
fn a_string_body_that_is_not_json_becomes_data() {
    let tool = tool("post", "/pets", BODY);
    let (request, _) = block_on(sent(&tool, r#"{"body":"hello"}"#));
    assert_eq!(body_json(&request), json(r#"{"data":"hello"}"#));
}

#[rstest]
#[case::zero(r#"{"body":0}"#)]
#[case::empty_string(r#"{"body":""}"#)]
#[case::empty_list(r#"{"body":[]}"#)]
#[case::null(r#"{"body":null}"#)]
fn a_falsy_non_object_body_sends_no_body(#[case] args: &str) {
    let (request, _) = block_on(sent(&tool("post", "/pets", BODY), args));
    assert_eq!(request.body, None);
}

#[test]
fn a_swagger_body_parameter_carries_the_body() {
    let tool = tool(
        "post",
        "/pets",
        r#"{"parameters":[{"name":"pet","in":"body"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"pet":{"a":1}}"#));
    assert_eq!(body_json(&request), json(r#"{"a":1}"#));
}

#[test]
fn a_named_body_parameter_defaults_to_an_empty_object() {
    let tool = tool(
        "post",
        "/pets",
        r#"{"parameters":[{"name":"pet","in":"body"}]}"#,
    );
    let (request, _) = block_on(sent(&tool, "{}"));
    assert_eq!(body_json(&request), json("{}"));
}

#[test]
fn the_body_argument_beats_a_named_body_parameter() {
    let tool = tool(
        "post",
        "/pets",
        r#"{"parameters":[{"name":"pet","in":"body"}],"requestBody":{"content":{"application/json":{}}}}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"body":{"b":2},"pet":{"a":1}}"#));
    assert_eq!(body_json(&request), json(r#"{"b":2}"#));
}

#[test]
fn a_falsy_body_falls_through_to_the_next_body_parameter() {
    let tool = tool(
        "post",
        "/pets",
        r#"{"parameters":[{"name":"pet","in":"body"}],"requestBody":{"content":{"application/json":{}}}}"#,
    );
    let (request, _) = block_on(sent(&tool, r#"{"body":{},"pet":{"a":1}}"#));
    assert_eq!(body_json(&request), json(r#"{"a":1}"#));
}

#[rstest]
#[case::get("get", None)]
#[case::delete("delete", None)]
#[case::post("post", Some(br#"{"a":1}"#.to_vec()))]
#[case::put("put", Some(br#"{"a":1}"#.to_vec()))]
#[case::patch("patch", Some(br#"{"a":1}"#.to_vec()))]
fn only_a_method_with_a_body_sends_one(#[case] method: &str, #[case] expected: Option<Vec<u8>>) {
    let (request, _) = block_on(sent(&tool(method, "/pets", BODY), r#"{"body":{"a":1}}"#));
    assert_eq!(request.body, expected);
}

#[rstest]
#[case::lower("options")]
#[case::upper("OPTIONS")]
#[case::head("head")]
fn an_unsupported_method_is_a_text_result(#[case] method: &str) {
    let transport = FakeTransport::replying(200, b"{}");
    let tool = tool(method, "/pets", "{}");
    let args = arguments("{}");
    let call_context = context();
    let call = tool.call(&args, &call_context, &transport);
    assert_eq!(
        block_on(call).unwrap(),
        text(&format!(
            "Unsupported HTTP method: {}",
            method.to_lowercase()
        ))
    );
    assert!(transport.requests().is_empty());
}

#[test]
fn static_headers_go_on_the_request() {
    let mut config = config("get", "/pets", "{}");
    config.headers = headers(&[("X-Tenant", "acme")]);
    let tool = OpenApiTool::new(config);
    let (request, _) = block_on(sent(&tool, "{}"));
    assert_eq!(request.headers, headers(&[("X-Tenant", "acme")]));
}

#[test]
fn the_context_carries_the_per_request_headers() {
    let tool = tool("get", "/pets", "{}");
    let context = OpenApiCallContext {
        auth_header: Some("Bearer byok".to_owned()),
        extra_headers: headers(&[("X-Trace", "t")]),
        resolved_auth_headers: headers(&[("X-Cred", "resolved")]),
    };
    let request = block_on(sent_with(&tool, "{}", &context));
    assert_eq!(
        request.headers,
        headers(&[
            ("Authorization", "Bearer byok"),
            ("X-Cred", "resolved"),
            ("X-Trace", "t"),
        ])
    );
}

#[test]
fn a_resolved_credential_outside_authorization_sets_the_redirect_guard() {
    let tool = tool("get", "/pets", "{}");
    let context = OpenApiCallContext {
        resolved_auth_headers: headers(&[("X-Cred", "resolved")]),
        auth_header: None,
        extra_headers: BTreeMap::new(),
    };
    let request = block_on(sent_with(&tool, "{}", &context));
    let guard = request.redirect_guard.unwrap();
    assert_eq!(guard.configured_url, format!("{BASE}/pets"));
    assert_eq!(guard.slot, "X-Cred");
}

#[test]
fn an_authorization_resolved_credential_needs_no_guard() {
    let tool = tool("get", "/pets", "{}");
    let context = OpenApiCallContext {
        resolved_auth_headers: headers(&[("authorization", "Bearer resolved")]),
        auth_header: None,
        extra_headers: BTreeMap::new(),
    };
    let request = block_on(sent_with(&tool, "{}", &context));
    assert_eq!(request.redirect_guard, None);
}

#[test]
fn no_resolved_credential_needs_no_guard() {
    let tool = tool("get", "/pets", "{}");
    let (request, _) = block_on(sent(&tool, "{}"));
    assert_eq!(request.redirect_guard, None);
}

#[test]
fn a_static_bearer_token_passes_the_credential_check() {
    let mut config = config("get", "/pets", "{}");
    config.auth_type = Some(McpAuthType::BearerToken);
    config.headers = headers(&[("Authorization", "Bearer abc")]);
    let tool = OpenApiTool::new(config);
    let (request, _) = block_on(sent(&tool, "{}"));
    assert_eq!(request.headers, headers(&[("Authorization", "Bearer abc")]));
}

#[test]
fn a_resolved_credential_passes_the_credential_check() {
    let mut config = config("get", "/pets", "{}");
    config.auth_type = Some(McpAuthType::BearerToken);
    let tool = OpenApiTool::new(config);
    let context = OpenApiCallContext {
        resolved_auth_headers: headers(&[("Authorization", "Bearer resolved")]),
        auth_header: None,
        extra_headers: BTreeMap::new(),
    };
    let request = block_on(sent_with(&tool, "{}", &context));
    assert_eq!(
        request.headers,
        headers(&[("Authorization", "Bearer resolved")])
    );
}

#[test]
fn a_missing_bearer_token_fails_before_sending() {
    let mut config = config("get", "/pets", "{}");
    config.auth_type = Some(McpAuthType::BearerToken);
    let tool = OpenApiTool::new(config);
    let error = block_on(failed_call(&tool, "{}"));
    assert!(matches!(
        error,
        OpenApiToolError::Credential(CredError::Misconfigured(message))
            if message == "bearer_token requires a usable upstream credential"
    ));
}

#[test]
fn a_bearer_scheme_without_a_credential_fails_before_sending() {
    let mut config = config("get", "/pets", "{}");
    config.auth_type = Some(McpAuthType::BearerToken);
    config.headers = headers(&[("Authorization", "Bearer ")]);
    let tool = OpenApiTool::new(config);
    let error = block_on(failed_call(&tool, "{}"));
    assert!(matches!(error, OpenApiToolError::Credential(_)));
}

#[test]
fn a_missing_api_key_fails_before_sending() {
    let mut config = config("get", "/pets", "{}");
    config.auth_type = Some(McpAuthType::ApiKey);
    let tool = OpenApiTool::new(config);
    let error = block_on(failed_call(&tool, "{}"));
    assert!(matches!(
        error,
        OpenApiToolError::Credential(CredError::Misconfigured(message))
            if message == "api_key requires a usable upstream credential"
    ));
}

#[test]
fn the_credential_check_runs_before_the_path_value() {
    let mut config = config(
        "get",
        "/repos/{owner}",
        r#"{"parameters":[{"name":"owner","in":"path"}]}"#,
    );
    config.auth_type = Some(McpAuthType::BearerToken);
    let tool = OpenApiTool::new(config);
    let error = block_on(failed_call(&tool, r#"{"owner":"a/b"}"#));
    assert!(matches!(error, OpenApiToolError::Credential(_)));
}

#[test]
fn auth_type_none_skips_the_credential_check() {
    let mut config = config("get", "/pets", "{}");
    config.auth_type = Some(McpAuthType::None);
    let tool = OpenApiTool::new(config);
    let (request, _) = block_on(sent(&tool, "{}"));
    assert_eq!(request.url, format!("{BASE}/pets"));
}

#[test]
fn the_server_label_reaches_the_upstream_error() {
    let mut config = config("get", "/pets", "{}");
    config.server_label = Some("petstore".to_owned());
    let tool = OpenApiTool::new(config);
    let transport = FakeTransport::replying(500, b"secret body");
    let error = block_on(async {
        tool.call(&arguments("{}"), &context(), &transport)
            .await
            .unwrap_err()
    });
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamStatus { status: 500, ref server } if server == "petstore"
    ));
}
