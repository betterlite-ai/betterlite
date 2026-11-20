use betterlite_http::interactions::base::{RequestFailure, RequestHeaders};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::api::video_character_transport::clients::CharacterBody;
use crate::api::video_character_transport::service::wire::{
    body_text, header_map, multipart_body, request_url, response_headers, status_failure,
};

#[test]
fn body_text_replaces_each_invalid_sequence() {
    assert_eq!(body_text(b"ok\xFFend"), "ok\u{FFFD}end");
    assert_eq!(body_text(b"\xE2\x28\xA1"), "\u{FFFD}(\u{FFFD}");
    assert_eq!(body_text(b"plain text"), "plain text");
    assert_eq!(body_text(b""), "");
}

#[test]
fn response_headers_joins_a_repeated_header() {
    let mut headers = HeaderMap::new();
    headers.append("set-cookie", HeaderValue::from_static("a=1"));
    headers.append("set-cookie", HeaderValue::from_static("b=2"));

    let mapped = response_headers(&headers);

    assert_eq!(
        mapped.get("set-cookie").map(String::as_str),
        Some("a=1, b=2")
    );
    assert_eq!(mapped.len(), 1);
}

#[test]
fn response_headers_reads_a_latin1_value() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-name",
        HeaderValue::from_bytes(b"caf\xE9").expect("a latin-1 byte is a header value"),
    );
    headers.insert("x-plain", HeaderValue::from_static("plain"));

    let mapped = response_headers(&headers);

    assert_eq!(mapped.get("x-name").map(String::as_str), Some("café"));
    assert_eq!(mapped.get("x-plain").map(String::as_str), Some("plain"));
}

#[test]
fn status_failure_carries_the_status_text_and_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", HeaderValue::from_static("30"));
    let reply = UpstreamResponse {
        status: 429,
        headers,
        body: b"slow down \xFF".to_vec(),
    };

    let RequestFailure::Status {
        status_code,
        headers,
        body,
    } = status_failure(&reply)
    else {
        panic!("a failing reply reads as a status failure");
    };

    assert_eq!(status_code, 429);
    assert_eq!(body, "slow down \u{FFFD}");
    assert_eq!(headers.get("retry-after").map(String::as_str), Some("30"));
}

#[test]
fn request_url_keeps_the_urls_own_value() {
    let query = vec![
        (String::from("expand"), String::from("param")),
        (String::from("view"), String::from("full")),
    ];

    let url = request_url("https://api.example/v1/characters/c?expand=url", &query).unwrap();

    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs,
        vec![
            (String::from("expand"), String::from("url")),
            (String::from("view"), String::from("full")),
        ]
    );
}

#[test]
fn request_url_keeps_repeated_params_in_the_source_order() {
    let query = vec![
        (String::from("tag"), String::from("a")),
        (String::from("tag"), String::from("b")),
    ];

    let url = request_url("https://api.example/v1/characters/c", &query).unwrap();

    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs,
        vec![
            (String::from("tag"), String::from("a")),
            (String::from("tag"), String::from("b")),
        ]
    );
}

#[test]
fn request_url_refuses_an_unparseable_url() {
    let error = request_url("not a url", &[]).unwrap_err();

    assert!(matches!(error, RequestFailure::Url(_)));
}

#[test]
fn multipart_body_marks_the_boundary_it_encoded() {
    let parts = vec![
        MultipartPart {
            name: String::from("name"),
            file_name: None,
            content_type: None,
            data: b"Wren".to_vec(),
        },
        MultipartPart {
            name: String::from("video"),
            file_name: Some(String::from("clip.mp4")),
            content_type: Some(String::from("video/mp4")),
            data: b"video-bytes".to_vec(),
        },
    ];

    let body = multipart_body(&parts, "b-1").unwrap();

    let CharacterBody::Multipart { content_type, data } = body else {
        panic!("a creation sends a multipart body");
    };
    assert_eq!(content_type, "multipart/form-data; boundary=b-1");
    let expected = String::from(
        "--b-1\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nWren\r\n--b-1\r\nContent-Disposition: form-data; name=\"video\"; filename=\"clip.mp4\"\r\nContent-Type: video/mp4\r\n\r\nvideo-bytes\r\n--b-1--\r\n",
    );
    assert_eq!(data, expected.into_bytes());
}

#[test]
fn multipart_body_refuses_a_part_that_holds_the_boundary() {
    let parts = vec![MultipartPart {
        name: String::from("video"),
        file_name: None,
        content_type: None,
        data: b"a--b-1--b".to_vec(),
    }];

    let error = multipart_body(&parts, "b-1").unwrap_err();

    let TransportError::MultipartBoundary { boundary } = error else {
        panic!("a part holding the boundary is refused");
    };
    assert_eq!(boundary, "b-1");
}

#[test]
fn multipart_body_refuses_a_content_type_with_a_line_break() {
    let parts = vec![MultipartPart {
        name: String::from("video"),
        file_name: None,
        content_type: Some(String::from("video/mp4\r\nx: y")),
        data: Vec::new(),
    }];

    let error = multipart_body(&parts, "b-1").unwrap_err();

    let TransportError::MultipartContentType { content_type } = error else {
        panic!("a content type with a line break is refused");
    };
    assert_eq!(content_type, "video/mp4\r\nx: y");
}

#[test]
fn header_map_refuses_a_value_with_a_newline() {
    let mut headers = RequestHeaders::new();
    headers.insert(
        String::from("x-bad"),
        SecretString::from(String::from("line\nbreak")),
    );

    let error = header_map(&headers).unwrap_err();

    assert!(matches!(error, RequestFailure::HeaderValue(_)));
}

#[test]
fn header_map_refuses_a_value_with_a_control_character() {
    let mut headers = RequestHeaders::new();
    headers.insert(
        String::from("x-bad"),
        SecretString::from(String::from("bell\u{7}")),
    );

    let error = header_map(&headers).unwrap_err();

    assert!(matches!(error, RequestFailure::HeaderValue(_)));
}

#[test]
fn header_map_lowercases_the_name_and_keeps_the_value() {
    let mut headers = RequestHeaders::new();
    headers.insert(
        String::from("Authorization"),
        SecretString::from(String::from("Bearer sk-1")),
    );

    let mapped = header_map(&headers).unwrap();

    assert_eq!(
        mapped
            .get("authorization")
            .map(|value| value.to_str().unwrap()),
        Some("Bearer sk-1")
    );
    assert_eq!(
        mapped
            .get("Authorization")
            .map(|value| value.to_str().unwrap()),
        Some("Bearer sk-1")
    );
}

#[test]
fn header_map_refuses_a_name_with_a_space() {
    let mut headers = RequestHeaders::new();
    headers.insert(
        String::from("x bad"),
        SecretString::from(String::from("value")),
    );

    let error = header_map(&headers).unwrap_err();

    assert!(matches!(error, RequestFailure::HeaderName(_)));
}
