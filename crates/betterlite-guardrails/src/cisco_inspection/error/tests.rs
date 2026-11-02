#![cfg(test)]

use std::error::Error as _;
use std::str;

use reqwest::header::HeaderValue;
use serde_json::value::RawValue;

use crate::cisco_inspection::error::ApiError;

#[test]
fn invalid_api_key_header_names_the_header_and_keeps_its_source() {
    let source =
        HeaderValue::from_bytes(b"\n").expect_err("a control byte is not a valid header value");
    let error = ApiError::InvalidApiKeyHeader { source };
    assert_eq!(
        error.to_string(),
        "the Cisco AI Defense API key header value is not valid"
    );
    assert!(error.source().is_some());
}

#[test]
fn invalid_user_agent_header_names_the_header_and_keeps_its_source() {
    let source =
        HeaderValue::from_bytes(b"\n").expect_err("a control byte is not a valid header value");
    let error = ApiError::InvalidUserAgentHeader { source };
    assert_eq!(
        error.to_string(),
        "the Cisco AI Defense user agent header value is not valid"
    );
    assert!(error.source().is_some());
}

#[test]
fn http_status_names_the_surface_status_and_snippet() {
    let error = ApiError::HttpStatus {
        surface: "chat".to_owned(),
        status: 400,
        snippet: "bad input".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "Cisco AI Defense chat API returned HTTP 400: bad input"
    );
}

#[test]
fn response_too_large_names_the_surface_and_limit() {
    let error = ApiError::ResponseTooLarge {
        surface: "mcp".to_owned(),
        limit: 1024,
    };
    assert_eq!(
        error.to_string(),
        "Cisco AI Defense mcp API answer exceeds 1024 bytes"
    );
}

#[test]
fn http_status_encoding_names_the_surface_and_status() {
    let bytes = vec![0xff_u8];
    let source = str::from_utf8(&bytes).expect_err("a lone continuation byte is not valid UTF-8");
    let error = ApiError::HttpStatusEncoding {
        surface: "mcp".to_owned(),
        status: 500,
        source,
    };
    assert_eq!(
        error.to_string(),
        "Cisco AI Defense mcp API returned HTTP 500 with invalid UTF-8"
    );
    assert!(error.source().is_some());
}

#[test]
fn response_json_keeps_the_parse_error_as_its_source() {
    let source = serde_json::from_str::<Box<RawValue>>("not json").expect_err("not valid JSON");
    let error = ApiError::ResponseJson {
        surface: "chat".to_owned(),
        source,
    };
    assert_eq!(
        error.to_string(),
        "Cisco AI Defense chat API returned a non-JSON response"
    );
    assert!(error.source().is_some());
}
