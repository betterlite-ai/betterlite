#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use rstest::rstest;

use crate::containers::container_file_http::proxied::{outbound_request, reply_headers};
use crate::containers::container_file_http::transport::{
    ContainerHttpRequest, ContainerMethod, TransferError, UploadedFile,
};

const DEADLINE: Duration = Duration::from_secs(600);

fn request(method: ContainerMethod) -> ContainerHttpRequest {
    ContainerHttpRequest {
        method,
        url: "https://h.test/v1/containers/c1/files?api-version=v1".to_owned(),
        headers: BTreeMap::from([("Authorization".to_owned(), "Bearer k".to_owned())]),
        query: vec![("limit".to_owned(), "5".to_owned())],
        file: None,
    }
}

#[rstest]
#[case::get(ContainerMethod::Get, Method::GET)]
#[case::delete(ContainerMethod::Delete, Method::DELETE)]
#[case::post(ContainerMethod::Post, Method::POST)]
fn outbound_request_maps_the_method_and_keeps_url_query_and_deadline(
    #[case] method: ContainerMethod,
    #[case] expected: Method,
) {
    let outbound = outbound_request(&request(method), DEADLINE).unwrap();
    assert_eq!(outbound.method, expected);
    assert_eq!(
        outbound.url.as_str(),
        "https://h.test/v1/containers/c1/files?api-version=v1"
    );
    assert_eq!(outbound.query, [("limit".to_owned(), "5".to_owned())]);
    assert_eq!(outbound.deadline, DEADLINE);
    assert_eq!(outbound.headers.get("authorization").unwrap(), "Bearer k");
    assert!(matches!(outbound.body, RequestBody::Empty));
}

#[test]
fn outbound_request_sends_the_file_as_one_multipart_part_named_file() {
    let mut upload = request(ContainerMethod::Post);
    upload.file = Some(UploadedFile {
        filename: "a.txt".to_owned(),
        content: b"hello".to_vec(),
        content_type: "text/plain".to_owned(),
    });
    let outbound = outbound_request(&upload, DEADLINE).unwrap();
    let RequestBody::Multipart { boundary, parts } = outbound.body else {
        panic!("expected a multipart body");
    };
    let [part] = parts.as_slice() else {
        panic!("expected one part, got {}", parts.len());
    };
    assert_eq!(part.name, "file");
    assert_eq!(part.file_name.as_deref(), Some("a.txt"));
    assert_eq!(part.content_type.as_deref(), Some("text/plain"));
    assert_eq!(part.data, b"hello");
    assert!(boundary.starts_with("betterlite-"));
}

#[test]
fn outbound_request_derives_a_different_boundary_for_different_content() {
    let boundary_of = |content: &[u8]| {
        let mut upload = request(ContainerMethod::Post);
        upload.file = Some(UploadedFile {
            filename: "a".to_owned(),
            content: content.to_vec(),
            content_type: "text/plain".to_owned(),
        });
        let RequestBody::Multipart { boundary, .. } =
            outbound_request(&upload, DEADLINE).unwrap().body
        else {
            panic!("expected a multipart body");
        };
        boundary
    };
    assert_eq!(boundary_of(b"one"), boundary_of(b"one"));
    assert_ne!(boundary_of(b"one"), boundary_of(b"two"));
}

#[test]
fn outbound_request_rejects_a_url_that_does_not_parse() {
    let mut bad = request(ContainerMethod::Get);
    bad.url = "not a url".to_owned();
    let error = outbound_request(&bad, DEADLINE).unwrap_err();
    assert!(matches!(error, TransferError::Url { url, .. } if url == "not a url"));
}

#[test]
fn outbound_request_rejects_a_header_name_that_is_not_valid() {
    let mut bad = request(ContainerMethod::Get);
    bad.headers.insert("bad name".to_owned(), "x".to_owned());
    let error = outbound_request(&bad, DEADLINE).unwrap_err();
    assert!(matches!(error, TransferError::HeaderName { name, .. } if name == "bad name"));
}

#[test]
fn outbound_request_rejects_a_header_value_with_a_line_break() {
    let mut bad = request(ContainerMethod::Get);
    bad.headers.insert("x-a".to_owned(), "one\ntwo".to_owned());
    let error = outbound_request(&bad, DEADLINE).unwrap_err();
    assert!(matches!(
        error,
        TransferError::Transport(TransportError::InvalidHeader { name, .. }) if name == "x-a"
    ));
}

#[test]
fn outbound_request_keeps_headers_that_differ_only_in_case() {
    let mut twice = request(ContainerMethod::Get);
    twice
        .headers
        .insert("authorization".to_owned(), "Bearer other".to_owned());
    let outbound = outbound_request(&twice, DEADLINE).unwrap();
    assert_eq!(outbound.headers.get_all("authorization").iter().count(), 2);
}

#[test]
fn reply_headers_lower_case_the_names_and_join_repeats_with_a_comma() {
    let mut headers = HeaderMap::new();
    headers.append(
        HeaderName::from_static("x-a"),
        HeaderValue::from_static("one"),
    );
    headers.append(
        HeaderName::from_static("x-a"),
        HeaderValue::from_static("two"),
    );
    headers.append(
        HeaderName::from_static("x-b"),
        HeaderValue::from_static("solo"),
    );
    assert_eq!(
        reply_headers(&headers),
        BTreeMap::from([
            ("x-a".to_owned(), "one, two".to_owned()),
            ("x-b".to_owned(), "solo".to_owned())
        ])
    );
}

#[test]
fn reply_headers_read_bytes_that_are_not_utf8_as_latin_1() {
    let mut headers = HeaderMap::new();
    headers.append(
        HeaderName::from_static("x-a"),
        HeaderValue::from_bytes(&[0x61, 0xff]).unwrap(),
    );
    assert_eq!(
        reply_headers(&headers).get("x-a").map(String::as_str),
        Some("a\u{ff}")
    );
}
