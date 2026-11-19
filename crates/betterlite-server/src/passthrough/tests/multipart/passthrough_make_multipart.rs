use std::time::Duration;

use betterlite_core::api::request_form::values::FormParts;
use betterlite_http::proxy::outbound::RequestBody;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap};

use super::{field, header_text, headers_with, row, rows_of, target, upload};
use crate::passthrough::multipart::make_multipart_http_request;

fn mixed_form() -> FormParts {
    FormParts {
        items: vec![
            field("mode", "fast"),
            upload("file", "a.txt", Some("text/plain"), b"AAA"),
            field("tag", "one"),
            field("mode", "slow"),
            upload("file", "b.txt", None, b"BBB"),
        ],
    }
}

#[test]
fn sends_grouped_fields_before_the_files() {
    let outbound = make_multipart_http_request(
        target(headers_with("multipart/form-data; boundary=caller")),
        &mixed_form(),
        "rebuilt",
    );
    let RequestBody::Multipart { boundary, parts } = outbound.body else {
        panic!("a form with parts must become a multipart body");
    };
    assert_eq!(boundary, "rebuilt");
    assert_eq!(
        rows_of(parts),
        vec![
            row("mode", None, b"fast"),
            row("mode", None, b"slow"),
            row("tag", None, b"one"),
            row("file", Some(("a.txt", "text/plain")), b"AAA"),
            row("file", Some(("b.txt", "text/plain")), b"BBB"),
        ]
    );
}

#[test]
fn the_rebuilt_request_keeps_the_method_query_and_other_headers() {
    let outbound = make_multipart_http_request(
        target(headers_with("multipart/form-data; boundary=caller")),
        &mixed_form(),
        "rebuilt",
    );
    assert_eq!(outbound.method, Method::POST);
    assert_eq!(outbound.headers.get(CONTENT_TYPE), None);
    assert_eq!(
        header_text(&outbound.headers, "x-custom").as_deref(),
        Some("kept")
    );
    assert_eq!(outbound.query, vec![("page".to_owned(), "2".to_owned())]);
    assert_eq!(outbound.deadline, Duration::from_secs(5));
}

#[test]
fn a_form_without_files_sends_every_field_in_order_through_files() {
    let form = FormParts {
        items: vec![field("a", "1"), field("b", "2"), field("a", "3")],
    };
    let outbound =
        make_multipart_http_request(target(headers_with("multipart/form-data")), &form, "b");
    let RequestBody::Multipart { parts, .. } = outbound.body else {
        panic!("a fileless form still becomes a multipart body");
    };
    assert_eq!(
        rows_of(parts),
        vec![
            row("a", None, b"1"),
            row("b", None, b"2"),
            row("a", None, b"3"),
        ]
    );
}

#[test]
fn an_empty_form_sends_no_body_at_all() {
    let outbound = make_multipart_http_request(
        target(headers_with("multipart/form-data; boundary=caller")),
        &FormParts::default(),
        "b",
    );
    assert!(matches!(outbound.body, RequestBody::Empty));
    assert_eq!(outbound.headers.get(CONTENT_TYPE), None);
    assert_eq!(
        header_text(&outbound.headers, "x-custom").as_deref(),
        Some("kept")
    );
}

#[test]
fn a_form_with_only_files_has_no_data_fields() {
    let form = FormParts {
        items: vec![upload("f", "a.txt", None, b"\x00\x01")],
    };
    let outbound =
        make_multipart_http_request(target(headers_with("multipart/form-data")), &form, "b");
    let RequestBody::Multipart { parts, .. } = outbound.body else {
        panic!("a form with a file becomes a multipart body");
    };
    assert_eq!(
        rows_of(parts),
        vec![row("f", Some(("a.txt", "text/plain")), b"\x00\x01")]
    );
}

#[test]
fn an_undeclared_part_type_is_guessed_from_the_filename() {
    let form = FormParts {
        items: vec![
            upload("kept", "a.txt", Some("application/pdf"), b"A"),
            upload("guessed", "b.txt", None, b"B"),
            upload("unknown", "noextension", None, b"C"),
            upload("unnamed", "", None, b"D"),
        ],
    };
    let outbound =
        make_multipart_http_request(target(headers_with("multipart/form-data")), &form, "b");
    let RequestBody::Multipart { parts, .. } = outbound.body else {
        panic!("a form with a file becomes a multipart body");
    };
    assert_eq!(
        rows_of(parts),
        vec![
            row("kept", Some(("a.txt", "application/pdf")), b"A"),
            row("guessed", Some(("b.txt", "text/plain")), b"B"),
            row(
                "unknown",
                Some(("noextension", "application/octet-stream")),
                b"C"
            ),
            row("unnamed", None, b"D"),
        ]
    );
}

#[test]
fn a_request_without_a_content_type_header_is_sent_as_is() {
    let form = FormParts {
        items: vec![field("a", "1")],
    };
    let outbound = make_multipart_http_request(target(HeaderMap::new()), &form, "b");
    assert_eq!(outbound.headers.get(CONTENT_TYPE), None);
    assert!(matches!(outbound.body, RequestBody::Multipart { .. }));
}
