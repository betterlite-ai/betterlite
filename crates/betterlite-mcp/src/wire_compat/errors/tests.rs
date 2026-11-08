#![cfg(test)]

use rstest::rstest;

use super::{
    OpenApiUpstreamError, ServerListError, ToolResultError, URL_CREDENTIALS_STATUS,
    UpstreamAuthError, url_credentials_exception,
};

fn auth_error(status_code: u16, challenge: Option<&str>) -> UpstreamAuthError {
    UpstreamAuthError {
        status_code,
        www_authenticate: challenge.map(str::to_owned),
        server_name: "github".to_owned(),
    }
}

#[rstest]
fn the_url_credentials_exception_carries_the_fixed_status_and_detail() {
    let exception = url_credentials_exception();
    assert_eq!(exception.status_code, 500);
    assert_eq!(URL_CREDENTIALS_STATUS, 500);
    assert_eq!(
        exception.detail,
        "misconfigured: auth_type none cannot be used with credentials embedded in the upstream URL; \
         remove them from the URL and configure Basic Auth with auth_type: basic and \
         auth_value: username:password"
    );
    assert!(exception.headers.is_empty());
}

#[rstest]
fn the_upstream_auth_message_names_the_server_and_status() {
    let error = auth_error(401, None);
    assert_eq!(
        error.to_string(),
        "Upstream MCP server 'github' returned 401"
    );
}

#[rstest]
fn a_401_without_a_challenge_fabricates_the_standard_resource_metadata_challenge() {
    let exception = auth_error(401, None).to_http_exception(Some("https://gateway.example"), None);
    assert_eq!(exception.status_code, 401);
    assert_eq!(exception.detail, "Unauthorized");
    assert_eq!(
        exception
            .headers
            .get("www-authenticate")
            .map(String::as_str),
        Some(
            "Bearer resource_metadata=\"https://gateway.example/.well-known/oauth-protected-resource/mcp/github\""
        )
    );
}

#[rstest]
fn a_trailing_slash_on_the_origin_is_stripped_from_the_challenge() {
    let exception = auth_error(401, None).to_http_exception(Some("https://gateway.example/"), None);
    assert_eq!(
        exception
            .headers
            .get("www-authenticate")
            .map(String::as_str),
        Some(
            "Bearer resource_metadata=\"https://gateway.example/.well-known/oauth-protected-resource/mcp/github\""
        )
    );
}

#[rstest]
fn a_legacy_server_route_fabricates_the_legacy_metadata_url() {
    let exception = auth_error(401, None).to_http_exception(
        Some("https://gateway.example"),
        Some("/github/mcp/messages"),
    );
    assert_eq!(
        exception
            .headers
            .get("www-authenticate")
            .map(String::as_str),
        Some(
            "Bearer resource_metadata=\"https://gateway.example/.well-known/oauth-protected-resource/github/mcp\""
        )
    );
}

#[rstest]
fn a_403_without_a_challenge_gets_the_forbidden_detail_and_no_challenge() {
    let exception = auth_error(403, None).to_http_exception(Some("https://gateway.example"), None);
    assert_eq!(exception.status_code, 403);
    assert_eq!(exception.detail, "Forbidden");
    assert!(exception.headers.is_empty());
}

#[rstest]
fn a_non_auth_status_keeps_the_unauthorized_detail_and_no_challenge() {
    let exception = auth_error(500, None).to_http_exception(Some("https://gateway.example"), None);
    assert_eq!(exception.status_code, 500);
    assert_eq!(exception.detail, "Unauthorized");
    assert!(exception.headers.is_empty());
}

#[rstest]
fn an_existing_challenge_is_forwarded_unchanged() {
    let exception = auth_error(401, Some("Bearer realm=\"upstream\""))
        .to_http_exception(Some("https://gateway.example"), None);
    assert_eq!(
        exception
            .headers
            .get("www-authenticate")
            .map(String::as_str),
        Some("Bearer realm=\"upstream\"")
    );
}

#[rstest]
fn a_missing_origin_fabricates_no_challenge() {
    let exception = auth_error(401, None).to_http_exception(None, None);
    assert_eq!(exception.status_code, 401);
    assert!(exception.headers.is_empty());
}

#[rstest]
fn an_empty_origin_fabricates_no_challenge() {
    let exception = auth_error(401, None).to_http_exception(Some(""), None);
    assert!(exception.headers.is_empty());
}

#[rstest]
fn the_openapi_upstream_error_reports_only_the_status() {
    let error = OpenApiUpstreamError {
        status_code: 502,
        server_name: "openapi".to_owned(),
    };
    assert_eq!(error.to_string(), "upstream returned HTTP 502");
    assert_eq!(error.server_name, "openapi");
}

#[rstest]
#[case::text("tool failed: bad input")]
#[case::empty("")]
fn the_tool_result_error_displays_the_message_it_was_built_with(#[case] message: &str) {
    let error = ToolResultError {
        message: message.to_owned(),
    };
    assert_eq!(error.to_string(), message);
}

#[rstest]
fn the_server_list_error_names_the_server_and_keeps_the_fault() {
    let error = ServerListError {
        fault: "timeout",
        server_name: "github".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "Listing tools from MCP server 'github' failed"
    );
    assert_eq!(error.fault, "timeout");
}
