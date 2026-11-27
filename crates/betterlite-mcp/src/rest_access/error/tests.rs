#![cfg(test)]

use rstest::rstest;

use crate::rest_access::error::{RestAccessError, RestDetail};

const MISSING_DATABASE: &str = "Database not available. Connect a database to your proxy";

#[rstest]
#[case::toolset_not_found("Toolset 'payments' not found", r#""Toolset 'payments' not found""#)]
#[case::scoped_to_no_servers(
    "API key is scoped to no MCP servers; toolset access is denied.",
    r#""API key is scoped to no MCP servers; toolset access is denied.""#
)]
#[case::toolset_not_granted(
    "API key does not have access to toolset 'toolset-1'.",
    r#""API key does not have access to toolset 'toolset-1'.""#
)]
fn detail_text_serializes_to_a_bare_json_string(#[case] text: &str, #[case] expected: &str) {
    let detail = RestDetail::Text(text.to_owned());
    assert_eq!(serde_json::to_string(&detail).unwrap(), expected);
}

#[test]
fn detail_error_serializes_to_an_object_with_only_error() {
    let detail = RestDetail::Error {
        error: MISSING_DATABASE.to_owned(),
    };
    assert_eq!(
        serde_json::to_string(&detail).unwrap(),
        r#"{"error":"Database not available. Connect a database to your proxy"}"#
    );
}

#[rstest]
#[case::ip_filtering(
    "ip_filtering",
    "MCP server 'srv-1' is not accessible from your IP address (203.0.113.7). This server is restricted to internal networks only. To make it externally accessible, set 'available_on_public_internet: true' in the server configuration.",
    r#"{"error":"ip_filtering","message":"MCP server 'srv-1' is not accessible from your IP address (203.0.113.7). This server is restricted to internal networks only. To make it externally accessible, set 'available_on_public_internet: true' in the server configuration."}"#
)]
#[case::server_not_found(
    "server_not_found",
    "MCP server 'srv-1' was not found",
    r#"{"error":"server_not_found","message":"MCP server 'srv-1' was not found"}"#
)]
#[case::access_denied(
    "access_denied",
    "The key is not allowed to access server srv-1",
    r#"{"error":"access_denied","message":"The key is not allowed to access server srv-1"}"#
)]
fn detail_error_message_serializes_error_before_message(
    #[case] error: &str,
    #[case] message: &str,
    #[case] expected: &str,
) {
    let detail = RestDetail::ErrorMessage {
        error: error.to_owned(),
        message: message.to_owned(),
    };
    assert_eq!(serde_json::to_string(&detail).unwrap(), expected);
}

#[rstest]
#[case::toolset_not_found("Toolset 'payments' not found")]
#[case::scoped_to_no_servers("API key is scoped to no MCP servers; toolset access is denied.")]
#[case::toolset_not_granted("API key does not have access to toolset 'toolset-1'.")]
fn display_text_detail_writes_the_bare_text(#[case] text: &str) {
    assert_eq!(RestDetail::Text(text.to_owned()).to_string(), text);
}

#[test]
fn display_error_detail_writes_compact_json() {
    let detail = RestDetail::Error {
        error: MISSING_DATABASE.to_owned(),
    };
    assert_eq!(
        detail.to_string(),
        r#"{"error":"Database not available. Connect a database to your proxy"}"#
    );
}

#[test]
fn display_error_message_detail_writes_compact_json_in_key_order() {
    let detail = RestDetail::ErrorMessage {
        error: "server_not_found".to_owned(),
        message: "MCP server 'srv-1' was not found".to_owned(),
    };
    assert_eq!(
        detail.to_string(),
        r#"{"error":"server_not_found","message":"MCP server 'srv-1' was not found"}"#
    );
}

#[rstest]
#[case::toolset_not_found(
    404,
    "Toolset 'payments' not found",
    "404: Toolset 'payments' not found"
)]
#[case::scoped_to_no_servers(
    403,
    "API key is scoped to no MCP servers; toolset access is denied.",
    "403: API key is scoped to no MCP servers; toolset access is denied."
)]
fn display_http_writes_the_status_and_the_plain_text(
    #[case] status: u16,
    #[case] text: &str,
    #[case] expected: &str,
) {
    let error = RestAccessError::Http {
        status,
        detail: RestDetail::Text(text.to_owned()),
    };
    assert_eq!(error.to_string(), expected);
}

#[test]
fn display_http_writes_the_status_then_the_compact_object() {
    let error = RestAccessError::Http {
        status: 500,
        detail: RestDetail::Error {
            error: MISSING_DATABASE.to_owned(),
        },
    };
    assert_eq!(
        error.to_string(),
        r#"500: {"error":"Database not available. Connect a database to your proxy"}"#
    );
}

#[test]
fn display_http_writes_the_status_then_the_compact_error_message() {
    let error = RestAccessError::Http {
        status: 403,
        detail: RestDetail::ErrorMessage {
            error: "access_denied".to_owned(),
            message: "The key is not allowed to access server srv-1".to_owned(),
        },
    };
    assert_eq!(
        error.to_string(),
        r#"403: {"error":"access_denied","message":"The key is not allowed to access server srv-1"}"#
    );
}

#[test]
fn display_port_writes_its_text_unchanged() {
    let error = RestAccessError::Port("database query failed: connection refused".to_owned());
    assert_eq!(
        error.to_string(),
        "database query failed: connection refused"
    );
}
