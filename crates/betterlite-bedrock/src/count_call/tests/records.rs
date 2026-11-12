use betterlite_http::proxy::UpstreamResponse;
use http::HeaderMap;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};

use crate::bedrock_settings::{ErrorHeaders, ResponseBody};
use crate::count_call::error::CountCallError;
use crate::count_call::record::{header_map, upstream_record};

use super::support::header_map as parsed;

const ENDPOINT: &str = "https://bedrock-runtime.us-west-2.amazonaws.com/model/m/count-tokens";

#[test]
fn count_call_authorization_header_is_sensitive() {
    let headers = header_map(&[
        ("Authorization".to_owned(), "Bearer fixture".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ])
    .unwrap();
    assert!(headers[AUTHORIZATION].is_sensitive());
    assert!(!headers[CONTENT_TYPE].is_sensitive());
}

#[test]
fn count_call_invalid_header_names_the_header() {
    let bad_name = header_map(&[("bad name".to_owned(), "x".to_owned())]).unwrap_err();
    assert!(matches!(&bad_name, CountCallError::Header { name, .. } if name == "bad name"));
    let bad_value = header_map(&[("x-a".to_owned(), "bad\nvalue".to_owned())]).unwrap_err();
    assert!(matches!(&bad_value, CountCallError::Header { name, .. } if name == "x-a"));
}

#[test]
fn count_call_upstream_record_keeps_the_response() {
    let mut headers = parsed(&[("x-amzn-requestid".to_owned(), "abc".to_owned())]);
    headers.insert("x-binary", HeaderValue::from_bytes(&[0xff]).unwrap());
    let response = UpstreamResponse {
        status: 503,
        headers,
        body: b"slow down".to_vec(),
    };
    let record = upstream_record(&response, ENDPOINT).unwrap();
    let kept = vec![("x-amzn-requestid".to_owned(), "abc".to_owned())];
    assert_eq!(record.error.status_code, 503);
    assert_eq!(record.error.message, "slow down");
    assert_eq!(record.response_headers, kept);
    assert_eq!(record.headers, Some(ErrorHeaders::Http(kept)));
    assert_eq!(
        record.response_body,
        ResponseBody::Read("slow down".to_owned())
    );
}

#[test]
fn count_call_upstream_record_refuses_non_utf8_bodies() {
    let response = UpstreamResponse {
        status: 500,
        headers: HeaderMap::new(),
        body: vec![0xff, 0xfe, 0xfd],
    };
    let record = upstream_record(&response, ENDPOINT).unwrap();
    assert_eq!(record.error.message, "3 bytes that are not UTF-8");
}
