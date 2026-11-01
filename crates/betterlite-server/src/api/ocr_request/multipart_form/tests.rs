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

#[tokio::test]
async fn parses_a_file_field_into_an_upload_document() {
    let body = multipart_body("X-BOUNDARY", b"%PDF-1.4", Some(("model", "mistral-ocr")));
    let form = parse_body(&body, "multipart/form-data; boundary=X-BOUNDARY")
        .await
        .unwrap();
    let (document, fields) = build(&form).unwrap();
    let Some(OcrDocument::Upload { bytes, mime_type }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(bytes, b"%PDF-1.4");
    assert_eq!(mime_type, Some("application/pdf".to_owned()));
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("mistral-ocr".to_owned()))
    );
}

#[tokio::test]
async fn strips_mime_type_parameters() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nhello\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
    let (document, _) = build(&form).unwrap();
    let Some(OcrDocument::Upload { mime_type, .. }) = document else {
        panic!("expected an upload document");
    };
    assert_eq!(mime_type, Some("text/plain".to_owned()));
}

#[tokio::test]
async fn octet_stream_mime_type_is_dropped() {
    let body = multipart_body("B", b"data", None);
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
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

#[tokio::test]
async fn missing_file_field_is_rejected() {
    let form = vec![FormField {
        name: "model".to_owned(),
        value: FormValue::Text("mistral-ocr".to_owned()),
    }];
    build(&form).unwrap_err();
}

#[tokio::test]
async fn empty_file_is_rejected() {
    let body = multipart_body("B", b"", None);
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
    build(&form).unwrap_err();
}

#[tokio::test]
async fn oversized_file_is_rejected() {
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

#[tokio::test]
async fn file_well_under_the_cap_is_accepted() {
    let bytes = vec![0_u8; 4096];
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

#[tokio::test]
async fn file_field_without_a_content_type_header_is_still_a_file() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n\r\ndata\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
    let Some(FormField {
        value: FormValue::File { content_type, .. },
        ..
    }) = form.first()
    else {
        panic!("expected a file field even without a content-type header");
    };
    assert_eq!(*content_type, None);
}

#[tokio::test]
async fn field_without_a_filename_but_with_a_content_type_is_still_a_file() {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"thumbnail\"\r\nContent-Type: image/png\r\n\r\ndata\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
    let Some(FormField {
        value: FormValue::File { filename, .. },
        ..
    }) = form.first()
    else {
        panic!("expected a file field even without a filename");
    };
    assert_eq!(*filename, None);
}

#[tokio::test]
async fn document_form_field_is_ignored() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(
        b"--B\r\nContent-Disposition: form-data; name=\"document\"\r\n\r\nignored\r\n--B--\r\n",
    );
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
    let (_, fields) = build(&form).unwrap();
    assert!(!fields.contains_key("document"));
}

#[tokio::test]
async fn non_json_field_is_kept_as_text() {
    let body = multipart_body("B", b"data", Some(("model", "mistral-ocr")));
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
    let (_, fields) = build(&form).unwrap();
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("mistral-ocr".to_owned()))
    );
}

#[tokio::test]
async fn non_utf8_text_field_is_rejected() {
    let mut body = Vec::new();
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\nContent-Type: application/pdf\r\n\r\ndata\r\n");
    body.extend_from_slice(b"--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(&[0xFF, 0xFE]);
    body.extend_from_slice(b"\r\n--B--\r\n");
    parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap_err();
}

#[tokio::test]
async fn json_field_is_decoded() {
    let body = multipart_body("B", b"data", Some(("pages", "[0,1,2]")));
    let form = parse_body(&body, "multipart/form-data; boundary=B")
        .await
        .unwrap();
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
