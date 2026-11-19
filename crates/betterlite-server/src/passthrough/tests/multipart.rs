use std::time::Duration;

use betterlite_core::api::request_form::values::{FormPart, PartValue, UploadFile};
use betterlite_http::proxy::multipart::MultipartPart;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::passthrough::upstream::UpstreamTarget;

fn field(name: &str, value: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Text(value.to_owned()),
    }
}

fn upload(name: &str, filename: &str, content_type: Option<&str>, content: &[u8]) -> FormPart {
    let uploaded = UploadFile {
        filename: filename.to_owned(),
        content_type: content_type.map(str::to_owned),
        content: content.to_vec(),
        size: Some(u64::try_from(content.len()).unwrap()),
    };
    FormPart {
        name: name.to_owned(),
        value: PartValue::Upload(uploaded),
    }
}

fn target(headers: HeaderMap) -> UpstreamTarget {
    UpstreamTarget {
        method: Method::POST,
        url: Url::parse("https://upstream.test/v1/files").unwrap(),
        headers,
        query: vec![("page".to_owned(), "2".to_owned())],
        deadline: Duration::from_secs(5),
    }
}

fn headers_with(content_type: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
    headers.insert(
        HeaderName::from_static("x-custom"),
        HeaderValue::from_static("kept"),
    );
    headers
}

#[derive(Debug, PartialEq, Eq)]
struct SentPart {
    name: String,
    file_name: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

fn rows_of(parts: Vec<MultipartPart>) -> Vec<SentPart> {
    parts
        .into_iter()
        .map(|part| SentPart {
            name: part.name,
            file_name: part.file_name,
            content_type: part.content_type,
            data: part.data,
        })
        .collect()
}

fn row(name: &str, part_type: Option<(&str, &str)>, data: &[u8]) -> SentPart {
    let (file_name, content_type) = part_type.unzip();
    SentPart {
        name: name.to_owned(),
        file_name: file_name.map(str::to_owned),
        content_type: content_type.map(str::to_owned),
        data: data.to_vec(),
    }
}

fn header_text(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(HeaderName::from_static(name))
        .map(|value| value.to_str().unwrap().to_owned())
}

mod passthrough_is_multipart {
    use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
    use rstest::rstest;

    use crate::passthrough::multipart::is_multipart;

    #[rstest]
    #[case::exact(b"multipart/form-data", true)]
    #[case::with_boundary(b"multipart/form-data; boundary=x", true)]
    #[case::inside_another_value(b"text/plain; note=multipart/form-data", true)]
    #[case::high_bytes(b"multipart/form-data\xff", true)]
    #[case::json(b"application/json", false)]
    #[case::urlencoded(b"application/x-www-form-urlencoded", false)]
    #[case::empty(b"", false)]
    fn looks_for_the_marker_inside_the_value(#[case] value: &[u8], #[case] expected: bool) {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_bytes(value).unwrap());
        assert_eq!(is_multipart(&headers), expected);
    }

    #[test]
    fn a_request_without_a_content_type_is_not_multipart() {
        assert!(!is_multipart(&HeaderMap::new()));
    }
}

mod passthrough_build_request_files {
    use betterlite_core::api::request_form::values::UploadFile;

    use crate::passthrough::multipart::{MultipartFile, request_file_from_upload};

    #[test]
    fn carries_the_upload_filename_content_and_content_type() {
        let upload = UploadFile {
            filename: "report.pdf".to_owned(),
            content_type: Some("application/pdf".to_owned()),
            content: b"%PDF".to_vec(),
            size: Some(4),
        };
        assert_eq!(
            request_file_from_upload("document", &upload),
            MultipartFile {
                field_name: "document".to_owned(),
                filename: Some("report.pdf".to_owned()),
                content: b"%PDF".to_vec(),
                content_type: Some("application/pdf".to_owned()),
            }
        );
    }

    #[test]
    fn an_upload_without_a_content_type_keeps_none() {
        let upload = UploadFile {
            filename: "notes".to_owned(),
            content_type: None,
            content: Vec::new(),
            size: None,
        };
        assert_eq!(
            request_file_from_upload("attach", &upload),
            MultipartFile {
                field_name: "attach".to_owned(),
                filename: Some("notes".to_owned()),
                content: Vec::new(),
                content_type: None,
            }
        );
    }
}

mod passthrough_make_multipart {
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
}
