#![cfg(test)]

use betterlite_ocr::json::Json;

use super::{build, parse_body};
use crate::api::ocr_request::{FormField, FormValue, OcrDocument};

fn multipart_body(boundary: &str, file_bytes: &[u8], extra_field: Option<(&str, &str)>) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");
    if let Some((name, value)) = extra_field {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[test]
fn parses_a_file_field_into_an_upload_document() {
    let body = multipart_body("X-BOUNDARY", b"%PDF-1.4", Some(("model", "mistral-ocr")));
    let form = parse_body(&body, "multipart/form-data; boundary=X-BOUNDARY").unwrap();
    let (document, fields) = build(&form).unwrap();
    let Some(OcrDocument::Upload {
        bytes,
        filename,
        mime_type,
    }) = document
    else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"%PDF-1.4");
    assert_eq!(filename, Some("a.pdf".to_owned()));
    assert_eq!(mime_type, Some("application/pdf".to_owned()));
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("mistral-ocr".to_owned()))
    );
}

#[test]
fn strips_mime_type_parameters() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nhello\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { mime_type, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(mime_type, Some("text/plain".to_owned()));
}

#[test]
fn octet_stream_mime_type_is_dropped() {
    let body = multipart_body("B", b"data", None);
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let mut form_with_octet_stream = form;
    if let Some(FormField {
        value: FormValue::File { content_type, .. },
        ..
    }) = form_with_octet_stream.first_mut()
    {
        *content_type = Some("application/octet-stream".to_owned());
    }
    let (document, _) = build(&form_with_octet_stream).unwrap();
    let Some(OcrDocument::Upload { mime_type, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(mime_type, None);
}

#[test]
fn missing_file_field_is_rejected() {
    let form = vec![FormField {
        name: "model".to_owned(),
        value: FormValue::Text("mistral-ocr".to_owned()),
    }];
    build(&form).unwrap_err();
}

#[test]
fn empty_file_is_rejected() {
    let body = multipart_body("B", b"", None);
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    build(&form).unwrap_err();
}

#[test]
fn oversized_file_is_rejected() {
    let oversized = vec![0_u8; 50 * 1024 * 1024 + 1];
    let form = vec![FormField {
        name: "file".to_owned(),
        value: FormValue::File {
            filename: Some("a.bin".to_owned()),
            content_type: None,
            bytes: oversized,
        },
    }];
    build(&form).unwrap_err();
}

#[test]
fn file_well_under_the_cap_is_accepted() {
    let bytes = vec![0_u8; 2_000_000];
    let form = vec![FormField {
        name: "file".to_owned(),
        value: FormValue::File {
            filename: Some("a.bin".to_owned()),
            content_type: None,
            bytes,
        },
    }];
    build(&form).unwrap();
}

#[test]
fn file_field_without_a_content_type_header_is_still_a_file() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n\r\ndata\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let Some(FormField {
        value: FormValue::File { content_type, .. },
        ..
    }) = form.first()
    else {
        panic!("expected a file field even without a content-type header");
    };
    assert_eq!(*content_type, None);
}

#[test]
fn field_without_a_filename_but_with_a_content_type_is_kept_as_text() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n",
    );
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"thumbnail\"\r\nContent-Type: image/png\r\n\r\ninline\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("thumbnail"),
        Some(&Json::Text("inline".to_owned()))
    );
}

#[test]
fn document_form_field_is_ignored() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\nignored\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert!(!fields.contains_key("document"));
}

#[test]
fn non_json_field_is_kept_as_text() {
    let body = multipart_body("B", b"data", Some(("model", "mistral-ocr")));
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("mistral-ocr".to_owned()))
    );
}

#[test]
fn non_utf8_text_field_falls_back_to_latin1() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0xFF, 0xFE]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("\u{ff}\u{fe}".to_owned()))
    );
}

#[test]
fn charset_param_picks_the_decoding() {
    // 0x80 is invalid UTF-8 either way, but encoding_rs's windows-1252 alias for "iso-8859-1" decodes it as U+20AC rather than true latin-1's U+0080.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0x80]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B; charset=iso-8859-1").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(fields.get("model"), Some(&Json::Text("\u{80}".to_owned())));
}

#[test]
fn us_ascii_charset_decodes_as_byte_identity() {
    // codecs.lookup("us-ascii") resolves to "ascii", so a byte above 0x7f falls back to latin-1's byte-identity decoding, not windows-1252's substitution.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0x80]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B; charset=us-ascii").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(fields.get("model"), Some(&Json::Text("\u{80}".to_owned())));
}

#[test]
fn charset_defaults_to_utf8_when_absent() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice("\u{e9}\u{e8}".as_bytes());
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("\u{e9}\u{e8}".to_owned()))
    );
}

#[test]
fn explicit_charset_decodes_a_multi_byte_sequence() {
    // The charset also decodes the field name, so a plain ASCII name does not survive utf-16le; read the field directly instead of going through build()'s name lookup.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0x41, 0x00]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B; charset=utf-16le").unwrap();
    let Some(FormField {
        value: FormValue::Text(text),
        ..
    }) = form.first()
    else {
        panic!("expected a text field");
    };
    assert_eq!(text, "A");
}

#[test]
fn a_part_without_a_name_is_rejected() {
    let body = b"--B\r\nContent-Disposition: form-data\r\n\r\nvalue\r\n--B--\r\n".to_vec();
    parse_body(&body, "multipart/form-data; boundary=B").unwrap_err();
}

#[test]
fn a_text_part_over_one_mebibyte_is_rejected() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&vec![b'a'; 1024 * 1024 + 1]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    parse_body(&body, "multipart/form-data; boundary=B").unwrap_err();
}

#[test]
fn a_text_part_at_one_mebibyte_is_accepted() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&vec![b'a'; 1024 * 1024]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    parse_body(&body, "multipart/form-data; boundary=B").unwrap();
}

fn parts_body(boundary: &str, count: usize, filename: Option<&str>) -> Vec<u8> {
    let disposition_suffix =
        filename.map_or_else(String::new, |name| format!("; filename=\"{name}\""));
    let mut body = Vec::new();
    for index in 0..count {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"f{index}\"{disposition_suffix}\r\n\r\nv\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[test]
fn one_thousand_fields_are_accepted() {
    let body = parts_body("B", 1000, None);
    parse_body(&body, "multipart/form-data; boundary=B").unwrap();
}

#[test]
fn one_thousand_and_one_fields_are_rejected() {
    let body = parts_body("B", 1001, None);
    parse_body(&body, "multipart/form-data; boundary=B").unwrap_err();
}

#[test]
fn one_thousand_files_are_accepted() {
    let body = parts_body("B", 1000, Some("a.bin"));
    parse_body(&body, "multipart/form-data; boundary=B").unwrap();
}

#[test]
fn one_thousand_and_one_files_are_rejected() {
    let body = parts_body("B", 1001, Some("a.bin"));
    parse_body(&body, "multipart/form-data; boundary=B").unwrap_err();
}

#[test]
fn the_last_field_named_file_wins() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"first.pdf\"\r\n\r\nfirst\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"second.pdf\"\r\n\r\nsecond\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"second");
}

#[test]
fn a_text_part_followed_by_an_upload_part_both_named_file_uses_the_upload() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\nnot a file\r\n",
    );
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\n\r\ndata\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"data");
}

#[test]
fn json_field_is_decoded() {
    let body = multipart_body("B", b"data", Some(("pages", "[0,1,2]")));
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("pages"),
        Some(&Json::List(vec![
            Json::Number(serde_json::Number::from(0_i64)),
            Json::Number(serde_json::Number::from(1_i64)),
            Json::Number(serde_json::Number::from(2_i64)),
        ]))
    );
}

#[test]
fn non_utf8_filename_falls_back_to_latin1() {
    // multipart-rs's own reader would reject this whole request as non-UTF-8; Starlette decodes a filename with the same charset-then-latin-1-fallback logic it uses for part bodies.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"");
    body.extend_from_slice(&[0xE9]);
    body.extend_from_slice(b".pdf\"\r\n\r\ndata\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { filename, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(filename, Some("\u{e9}.pdf".to_owned()));
}

#[test]
fn non_utf8_field_name_falls_back_to_latin1() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"");
    body.extend_from_slice(&[0xE9]);
    body.extend_from_slice(b"\"\r\n\r\nvalue\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(fields.get("\u{e9}"), Some(&Json::Text("value".to_owned())));
}

#[test]
fn windows_path_filename_keeps_only_the_last_segment() {
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"C:\\Users\\a\\scan.pdf\"\r\n\r\ndata\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { filename, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(filename, Some("scan.pdf".to_owned()));
}

#[test]
fn escaped_quote_in_filename_is_unescaped() {
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a\\\"b.pdf\"\r\n\r\ndata\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { filename, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(filename, Some("a\"b.pdf".to_owned()));
}
