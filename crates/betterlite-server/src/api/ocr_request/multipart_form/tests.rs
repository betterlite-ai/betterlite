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
fn closing_boundary_with_no_trailing_crlf_still_ends_the_file() {
    // The closing boundary line's second hyphen can be the body's literal last byte, with no CRLF after it, so the last part's data must not keep the boundary line.
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\n\r\ndata\r\n--B--".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"data");
}

#[test]
fn a_data_line_that_only_starts_with_the_boundary_stays_in_the_content() {
    // "--Bxyz" merely starts with the dash-boundary "--B"; a real boundary line needs dash_boundary followed by CRLF or "--", so this line is ordinary content.
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\n\r\ndata\r\n--Bxyz\r\nmore\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"data\r\n--Bxyz\r\nmore");
}

#[test]
fn closing_boundary_followed_by_trailing_characters_still_ends_the_file() {
    // python-multipart stops at the closing boundary's second hyphen and drops the rest of that line (a trailing space or an epilogue), so the file must not pick up "-- \r\n" as part of its data.
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\n\r\ndata\r\n--B-- \r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"data");
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
    // 0xE9 under windows-1251 is "\u{439}" (й), while the utf-8-then-latin-1 fallback path would give "\u{e9}": a mutant that ignores charset_param and always decodes as utf-8 still fails to decode 0xE9, but lands on the wrong character.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0xE9]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(
        &body,
        "multipart/form-data; boundary=B; charset=windows-1251",
    )
    .unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(fields.get("model"), Some(&Json::Text("\u{439}".to_owned())));
}

#[test]
fn iso_8859_1_decodes_as_true_latin1_not_windows_1252() {
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
fn iso_8859_9_keeps_c1_controls_byte_identical_not_windows_1254() {
    // 0x80 is invalid UTF-8 either way, but encoding_rs's windows-1254 alias for "iso-8859-9" leaves it undefined rather than true iso8859_9's byte-identical U+0080.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0x80]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B; charset=iso-8859-9").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(fields.get("model"), Some(&Json::Text("\u{80}".to_owned())));
}

#[test]
fn gb2312_decodes_through_encoding_rs_with_no_strict_table() {
    // 0x81 0x40 has no mapping in Python's strict gb2312 codec, but gb2312 is not one of the ascii/latin-1/iso8859-9 labels, so it now falls through to encoding_rs's gb2312 label (an alias for GBK), decoding to U+4E02 instead of falling back to latin-1.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0x81, 0x40]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    let form = parse_body(&body, "multipart/form-data; boundary=B; charset=gb2312").unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("\u{4e02}".to_owned()))
    );
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
fn split_items_yields_the_first_part_before_the_second_parts_bad_header_fails() {
    // A split_items that collects every part into a Vec first would fail on the broken second part before returning anything; reading one item at a time must hand back the first part's Ok before the second part is ever read.
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nv\r\n");
    body.extend_from_slice(b"--B\r\nBad-Header-With-No-Colon-Space\r\n\r\nv\r\n");
    body.extend_from_slice(b"--B--\r\n");
    let mut items = super::raw_split::split_items(&body, "B");
    assert!(matches!(items.next(), Some(Ok(_))));
    assert!(matches!(items.next(), Some(Err(_))));
}

fn mixed_parts_body(boundary: &str, field_count: usize, file_count: usize) -> Vec<u8> {
    let mut body = Vec::new();
    for index in 0..field_count {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"f{index}\"\r\n\r\nv\r\n"
            )
            .as_bytes(),
        );
    }
    for index in 0..file_count {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"u{index}\"; filename=\"a{index}.bin\"\r\n\r\nv\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[test]
fn exactly_the_combined_cap_is_accepted() {
    // 1000 fields and 1000 files each land exactly on their own cap at once, proving parse_body's field and file counters are independent rather than sharing one combined budget.
    let body = mixed_parts_body("B", 1000, 1000);
    parse_body(&body, "multipart/form-data; boundary=B").unwrap();
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

#[test]
fn semicolon_inside_quoted_filename_does_not_end_the_parameter() {
    // parse_options_header only splits on a ';' outside a quoted string, so "report;v2.pdf" stays one filename rather than being cut at the first semicolon.
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"report;v2.pdf\"\r\n\r\ndata\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { filename, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(filename, Some("report;v2.pdf".to_owned()));
}

#[test]
fn disposition_param_key_is_case_and_space_insensitive() {
    // parse_options_header lowercases and trims each parameter key, so "NAME" and extra spaces around '=' are accepted the same as "name".
    let body = b"--B\r\nContent-Disposition: form-data; NAME = \"file\"; filename=\"a.pdf\"\r\n\r\ndata\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    assert!(document.is_some());
}

#[test]
fn a_repeated_name_parameter_keeps_its_last_value() {
    // parse_options_header builds a dict from the parameters, so a later "name" overwrites an earlier one; here the field ends up named "file" and becomes the upload document.
    let body = b"--B\r\nContent-Disposition: form-data; name=\"x\"; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"%PDF-1.4");
}

#[test]
fn a_repeated_filename_parameter_keeps_its_last_value() {
    // The same last-value-wins rule applies to filename: a later "filename" overwrites an earlier one.
    let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"; filename=\"b.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4\r\n--B--\r\n".to_vec();
    let form = parse_body(&body, "multipart/form-data; boundary=B").unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { filename, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(filename, Some("b.pdf".to_owned()));
}
