#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::api::request_body::{JsonObject, ParsedJson, parse_json_text};
use crate::api::request_form::values::{FormPart, FormParts, PartValue, UploadFile};
use crate::rag::upload_security::{EICAR_TEST_SIGNATURE, EicarTestMalwareScanner, RejectionReason};

use super::base64::{Base64DecodeError, decode};
use super::error::IngestRequestError;
use super::{
    IngestBody, IngestRequest, ParsedIngestRequest, SecuredDocument, parse_rag_ingest_request,
};

const ID: uuid::Uuid = uuid::Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);
const SAFE_NAME: &str = "550e8400e29b41d4a716446655440000";
const LIMIT: usize = 16;

fn request_headers(content_type: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("content-type".to_owned(), content_type.to_owned())])
}

fn json_object(body: &str) -> JsonObject {
    match parse_json_text(body) {
        ParsedJson::Object(data) => data,
        ParsedJson::NotObject | ParsedJson::Invalid => {
            panic!("the test body must be a JSON object")
        }
    }
}

fn document(body: &str) -> Result<ParsedIngestRequest, IngestRequestError> {
    let data = json_object(body);
    let headers = request_headers("application/json");
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Json(&data),
    };
    parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID)
}

fn form(items: Vec<FormPart>) -> Result<ParsedIngestRequest, IngestRequestError> {
    let parts = FormParts { items };
    let headers = request_headers("multipart/form-data; boundary=betterlite");
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Multipart(&parts),
    };
    parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID)
}

fn text_part(name: &str, text: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Text(text.to_owned()),
    }
}

fn upload_part(name: &str, filename: &str, content: &[u8]) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Upload(UploadFile {
            filename: filename.to_owned(),
            content_type: Some("application/octet-stream".to_owned()),
            content: content.to_vec(),
            size: None,
        }),
    }
}

fn secured(filename: &str, content: &[u8], content_type: &str) -> SecuredDocument {
    SecuredDocument {
        safe_filename: filename.to_owned(),
        content: content.to_vec(),
        content_type: content_type.to_owned(),
    }
}

const STORE: &str = r#""ingest_options": {"vector_store": {"vector_store_id": "vs-1"}}"#;

#[test]
fn multipart_file_upload() {
    let request = text_part(
        "request",
        r#"{"ingest_options": {"vector_store": {"vector_store_id": "vs-1"}}}"#,
    );
    let file = upload_part("file", "notes.txt", b"hello\n");
    let parsed = form(vec![file, request]).unwrap();
    assert_eq!(
        parsed.file,
        Some(secured(
            &format!("{SAFE_NAME}.txt"),
            b"hello\n",
            "text/plain"
        ))
    );
    assert_eq!(parsed.file_url, None);
    assert_eq!(parsed.file_id, None);
    assert!(parsed.ingest_options.contains_key("vector_store"));
}

#[test]
fn multipart_missing_file() {
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    assert_eq!(form(vec![request]), Err(IngestRequestError::MissingSource));
}

#[test]
fn multipart_text_file_field_is_no_file() {
    let file = text_part("file", "hello");
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    assert_eq!(
        form(vec![file, request]),
        Err(IngestRequestError::MissingSource)
    );
}

#[test]
fn multipart_file_list_is_no_file() {
    let file = text_part("file[]", "hello");
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    assert_eq!(
        form(vec![file, request]),
        Err(IngestRequestError::MissingSource)
    );
}

#[test]
fn multipart_request_upload_is_not_text() {
    let file = upload_part("file", "notes.txt", b"hello\n");
    let request = upload_part("request", "request.json", b"{}");
    assert_eq!(
        form(vec![file, request]),
        Err(IngestRequestError::RequestNotText)
    );
}

#[test]
fn multipart_request_list_is_not_text() {
    let file = upload_part("file", "notes.txt", b"hello\n");
    let request = text_part("request[]", r#"{"ingest_options": {"vector_store": {}}}"#);
    assert_eq!(
        form(vec![file, request]),
        Err(IngestRequestError::RequestNotText)
    );
}

#[test]
fn multipart_request_json_supplies_url_and_id() {
    let request = text_part(
        "request",
        r#"{"ingest_options": {"vector_store": {}}, "file_url": "https://example.test/doc.pdf", "file_id": "file-abc"}"#,
    );
    let parsed = form(vec![request]).unwrap();
    assert_eq!(parsed.file, None);
    assert_eq!(
        parsed.file_url.as_deref(),
        Some("https://example.test/doc.pdf")
    );
    assert_eq!(parsed.file_id.as_deref(), Some("file-abc"));
}

#[test]
fn multipart_request_invalid_json() {
    let file = upload_part("file", "notes.txt", b"hello\n");
    let request = text_part("request", "{");
    assert_eq!(
        form(vec![file, request]),
        Err(IngestRequestError::RequestInvalidJson)
    );
}

#[test]
fn multipart_request_not_object() {
    let request = text_part("request", "[]");
    assert_eq!(
        form(vec![request]),
        Err(IngestRequestError::RequestNotObject)
    );
}

#[test]
fn multipart_request_empty_text_is_parsed() {
    let request = text_part("request", "");
    assert_eq!(
        form(vec![request]),
        Err(IngestRequestError::RequestInvalidJson)
    );
}

#[test]
fn multipart_without_request_field_has_no_options() {
    let file = upload_part("file", "notes.txt", b"hello\n");
    assert_eq!(
        form(vec![file]),
        Err(IngestRequestError::MissingVectorStore)
    );
}

#[test]
fn json_body_with_url() {
    let body = format!(r#"{{{STORE}, "file_url": "https://example.test/doc.pdf"}}"#);
    let parsed = document(&body).unwrap();
    assert_eq!(parsed.file, None);
    assert_eq!(
        parsed.file_url.as_deref(),
        Some("https://example.test/doc.pdf")
    );
    assert_eq!(parsed.file_id, None);
    assert!(parsed.ingest_options.contains_key("vector_store"));
}

#[test]
fn json_body_with_file_id() {
    let body = format!(r#"{{{STORE}, "file_id": "file-abc"}}"#);
    let parsed = document(&body).unwrap();
    assert_eq!(parsed.file, None);
    assert_eq!(parsed.file_url, None);
    assert_eq!(parsed.file_id.as_deref(), Some("file-abc"));
    assert!(parsed.ingest_options.contains_key("vector_store"));
}

#[test]
fn json_with_base64_file() {
    let body = format!(r#"{{{STORE}, "file": {{"filename": "doc.txt", "content": "aGVsbG8="}}}}"#);
    let parsed = document(&body).unwrap();
    assert_eq!(
        parsed.file,
        Some(secured(&format!("{SAFE_NAME}.txt"), b"hello", "text/plain"))
    );
    assert_eq!(parsed.file_url, None);
    assert_eq!(parsed.file_id, None);
}

#[test]
fn json_base64_pdf_keeps_the_pdf_extension() {
    let body =
        format!(r#"{{{STORE}, "file": {{"filename": "doc.pdf", "content": "JVBERi0xLjcK"}}}}"#);
    let parsed = document(&body).unwrap();
    assert_eq!(
        parsed.file,
        Some(secured(
            &format!("{SAFE_NAME}.pdf"),
            b"%PDF-1.7\n",
            "application/pdf"
        ))
    );
}

#[test]
fn json_file_ignores_the_client_content_type() {
    let body = format!(
        r#"{{{STORE}, "file": {{"filename": "doc.txt", "content": "aGVsbG8=", "content_type": "image/png"}}}}"#
    );
    let parsed = document(&body).unwrap();
    assert_eq!(
        parsed.file,
        Some(secured(&format!("{SAFE_NAME}.txt"), b"hello", "text/plain"))
    );
}

#[test]
fn json_file_without_filename_is_no_file() {
    let body = format!(
        r#"{{{STORE}, "file": {{"content": "aGVsbG8="}}, "file_url": "https://example.test/doc.pdf"}}"#
    );
    let parsed = document(&body).unwrap();
    assert_eq!(parsed.file, None);
    assert_eq!(
        parsed.file_url.as_deref(),
        Some("https://example.test/doc.pdf")
    );
}

#[test]
fn json_file_without_content_is_no_file() {
    let body = format!(
        r#"{{{STORE}, "file": {{"filename": "doc.txt"}}, "file_url": "https://example.test/doc.pdf"}}"#
    );
    let parsed = document(&body).unwrap();
    assert_eq!(parsed.file, None);
}

#[test]
fn json_file_null_content_is_no_file() {
    let body = format!(
        r#"{{{STORE}, "file": {{"filename": "doc.txt", "content": null}}, "file_url": "https://example.test/doc.pdf"}}"#
    );
    let parsed = document(&body).unwrap();
    assert_eq!(parsed.file, None);
}

#[test]
fn invalid_base64_content() {
    let body = format!(r#"{{{STORE}, "file": {{"filename": "doc.txt", "content": "a"}}}}"#);
    assert_eq!(
        document(&body),
        Err(IngestRequestError::InvalidBase64 {
            source: Base64DecodeError::OneModuloFour { characters: 1 },
        })
    );
    assert_eq!(
        document(&body).unwrap_err().to_string(),
        "Invalid base64 content: Invalid base64-encoded string: number of data characters (1) cannot be 1 more than a multiple of 4"
    );
}

#[test]
fn invalid_base64_padding() {
    let body = format!(r#"{{{STORE}, "file": {{"filename": "doc.txt", "content": "aQ"}}}}"#);
    assert_eq!(
        document(&body),
        Err(IngestRequestError::InvalidBase64 {
            source: Base64DecodeError::IncorrectPadding,
        })
    );
}

#[test]
fn invalid_base64_non_ascii() {
    let body =
        format!("{{{STORE}, \"file\": {{\"filename\": \"doc.txt\", \"content\": \"\\u00e9\"}}}}");
    assert_eq!(
        document(&body),
        Err(IngestRequestError::InvalidBase64 {
            source: Base64DecodeError::NonAscii,
        })
    );
}

#[test]
fn missing_all_sources() {
    assert_eq!(
        document(r#"{"ingest_options": {"vector_store": {}}}"#),
        Err(IngestRequestError::MissingSource)
    );
}

#[test]
fn missing_vector_store() {
    assert_eq!(
        document(r#"{"file_url": "https://example.test/doc.pdf"}"#),
        Err(IngestRequestError::MissingVectorStore)
    );
    assert_eq!(
        document(r#"{"file_url": "https://example.test/doc.pdf"}"#)
            .unwrap_err()
            .to_string(),
        "ingest_options must contain 'vector_store' configuration"
    );
}

#[rstest::rstest]
#[case::vertex_credentials("vertex_credentials")]
#[case::vertex_ai_credentials("vertex_ai_credentials")]
#[case::aws_access_key_id("aws_access_key_id")]
#[case::aws_secret_access_key("aws_secret_access_key")]
#[case::aws_session_token("aws_session_token")]
#[case::aws_web_identity_token("aws_web_identity_token")]
#[case::aws_role_name("aws_role_name")]
#[case::aws_session_name("aws_session_name")]
#[case::aws_profile_name("aws_profile_name")]
#[case::aws_sts_endpoint("aws_sts_endpoint")]
#[case::aws_external_id("aws_external_id")]
#[case::azure_ad_token("azure_ad_token")]
#[case::api_key("api_key")]
#[case::api_base("api_base")]
fn blocked_credential_field(#[case] field: &'static str) {
    let body = format!(
        r#"{{"ingest_options": {{"vector_store": {{"{field}": "x"}}}}, "file_url": "https://example.test/doc.pdf"}}"#
    );
    assert_eq!(
        document(&body),
        Err(IngestRequestError::BlockedCredential { field })
    );
}

#[test]
fn blocked_credential_message() {
    let body = r#"{"ingest_options": {"vector_store": {"vertex_credentials": "{}"}}, "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(
        document(body).unwrap_err().to_string(),
        "'vertex_credentials' cannot be set in ingest_options.vector_store. Credentials must be configured server-side."
    );
}

#[test]
fn blocked_credential_order() {
    let first = r#"{"ingest_options": {"vector_store": {"api_base": "https://evil.test", "vertex_credentials": "{}"}}, "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(
        document(first),
        Err(IngestRequestError::BlockedCredential {
            field: "vertex_credentials"
        })
    );
    let second = r#"{"ingest_options": {"vector_store": {"api_base": "https://evil.test", "api_key": "secret"}}, "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(
        document(second),
        Err(IngestRequestError::BlockedCredential { field: "api_key" })
    );
}

#[test]
fn blocked_credential_skips_non_object_vector_store() {
    let body = r#"{"ingest_options": {"vector_store": "vertex_credentials"}, "file_url": "https://example.test/doc.pdf"}"#;
    let parsed = document(body).unwrap();
    assert!(parsed.ingest_options.contains_key("vector_store"));
}

#[test]
fn vector_store_null_is_present() {
    let body =
        r#"{"ingest_options": {"vector_store": null}, "file_url": "https://example.test/doc.pdf"}"#;
    let parsed = document(body).unwrap();
    assert!(parsed.ingest_options.contains_key("vector_store"));
}

#[test]
fn ingest_options_null_is_absent() {
    let body = r#"{"ingest_options": null, "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(document(body), Err(IngestRequestError::MissingVectorStore));
}

#[test]
fn ingest_options_must_be_an_object() {
    let body = r#"{"ingest_options": [], "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(
        document(body),
        Err(IngestRequestError::NotAnObject {
            field: "ingest_options"
        })
    );
}

#[test]
fn file_must_be_an_object() {
    let body = r#"{"file": "doc", "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(
        document(body),
        Err(IngestRequestError::NotAnObject { field: "file" })
    );
}

#[test]
fn file_url_must_be_text() {
    assert_eq!(
        document(r#"{"file_url": 5}"#),
        Err(IngestRequestError::NotText { field: "file_url" })
    );
}

#[test]
fn file_id_must_be_text() {
    assert_eq!(
        document(r#"{"file_id": 5}"#),
        Err(IngestRequestError::NotText { field: "file_id" })
    );
}

#[test]
fn file_url_empty_string_is_a_source() {
    let body = r#"{"ingest_options": {"vector_store": {}}, "file_url": ""}"#;
    let parsed = document(body).unwrap();
    assert_eq!(parsed.file_url.as_deref(), Some(""));
}

#[test]
fn missing_source_is_reported_before_vector_store() {
    assert_eq!(document("{}"), Err(IngestRequestError::MissingSource));
}

#[test]
fn base64_error_is_reported_before_source_check() {
    let body = r#"{"file": {"filename": "doc.txt", "content": "a"}, "file_url": "https://example.test/doc.pdf"}"#;
    assert_eq!(
        document(body),
        Err(IngestRequestError::InvalidBase64 {
            source: Base64DecodeError::OneModuloFour { characters: 1 },
        })
    );
}

#[test]
fn upload_is_validated_before_vector_store() {
    let file = upload_part("file", "eicar.txt", EICAR_TEST_SIGNATURE);
    let request = text_part("request", r#"{"ingest_options": {}}"#);
    let error = form(vec![file, request]).unwrap_err();
    let IngestRequestError::UploadRejected { upload } = error else {
        panic!("the upload must be validated before the vector store is read");
    };
    assert_eq!(upload.reason, RejectionReason::MalwareDetected);
}

#[test]
fn upload_is_validated_before_credentials() {
    let file = upload_part("file", "empty.txt", b"");
    let request = text_part(
        "request",
        r#"{"ingest_options": {"vector_store": {"api_key": "secret"}}}"#,
    );
    let error = form(vec![file, request]).unwrap_err();
    let IngestRequestError::UploadRejected { upload } = error else {
        panic!("the upload must be validated before the credential fields are read");
    };
    assert_eq!(upload.reason, RejectionReason::EmptyFile);
}

#[test]
fn file_at_the_limit_is_secured() {
    let file = upload_part("file", "notes.txt", b"0123456789abcdef");
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    let parsed = form(vec![file, request]).unwrap();
    assert_eq!(parsed.file.unwrap().content.len(), LIMIT);
}

#[test]
fn file_over_the_limit_is_rejected() {
    let file = upload_part("file", "notes.txt", b"0123456789abcdefg");
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    let error = form(vec![file, request]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Uploaded file is 17 bytes, exceeding the 16-byte limit."
    );
}

#[test]
fn file_reading_stops_one_byte_past_the_limit() {
    let file = upload_part("file", "big.txt", &[b'a'; 100]);
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    let error = form(vec![file, request]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Uploaded file is 17 bytes, exceeding the 16-byte limit."
    );
}

#[test]
fn empty_file_is_rejected() {
    let file = upload_part("file", "empty.txt", b"");
    let request = text_part("request", r#"{"ingest_options": {"vector_store": {}}}"#);
    assert_eq!(
        form(vec![file, request]).unwrap_err().to_string(),
        "Uploaded file is empty."
    );
}

#[test]
fn multipart_marker_in_any_header_selects_the_multipart_branch() {
    let data = json_object(r#"{"ingest_options": {"vector_store": {}}}"#);
    let headers = request_headers("application/json; note=multipart/form-data");
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Json(&data),
    };
    assert_eq!(
        parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID),
        Err(IngestRequestError::BodyKindMismatch)
    );
}

#[test]
fn multipart_body_under_a_json_content_type_is_a_mismatch() {
    let parts = FormParts {
        items: vec![upload_part("file", "notes.txt", b"hello\n")],
    };
    let headers = request_headers("application/json");
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Multipart(&parts),
    };
    assert_eq!(
        parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID),
        Err(IngestRequestError::BodyKindMismatch)
    );
}

#[test]
fn content_type_match_is_case_sensitive() {
    let parts = FormParts {
        items: vec![upload_part("file", "notes.txt", b"hello\n")],
    };
    let headers = request_headers("Multipart/Form-Data; boundary=betterlite");
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Multipart(&parts),
    };
    assert_eq!(
        parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID),
        Err(IngestRequestError::BodyKindMismatch)
    );
}

#[test]
fn missing_content_type_takes_the_json_branch() {
    let data = json_object(r#"{"ingest_options": {"vector_store": {}}, "file_id": "file-abc"}"#);
    let headers = BTreeMap::new();
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Json(&data),
    };
    let parsed =
        parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID).unwrap();
    assert_eq!(parsed.file_id.as_deref(), Some("file-abc"));
}

#[rstest::rstest]
#[case::quad("QUJD", Some(b"ABC".as_slice()))]
#[case::one_byte("QQ==", Some(b"A".as_slice()))]
#[case::two_bytes("QUI=", Some(b"AB".as_slice()))]
#[case::three_bytes_with_one_pad("QUJ=", Some(b"AB".as_slice()))]
#[case::four_chars_and_two_pads("QUJDRA==", Some(b"ABCD".as_slice()))]
#[case::extra_pads_after_two_data_chars("QQ===", Some(b"A".as_slice()))]
#[case::extra_pads_after_three_data_chars("QUJ==", Some(b"AB".as_slice()))]
#[case::plus_and_slash("+/+/", Some([0xfb_u8, 0xff, 0xbf].as_slice()))]
#[case::digits("0000", Some([0xd3_u8, 0x4d, 0x34].as_slice()))]
#[case::lowercase("aaaa", Some([0x69_u8, 0xa6, 0x9a].as_slice()))]
#[case::uppercase("ZZZZ", Some([101_u8, 150, 89].as_slice()))]
#[case::empty("", Some(b"".as_slice()))]
#[case::only_off_alphabet("!!", Some(b"".as_slice()))]
#[case::off_alphabet_inside("aGVs*bG8=", Some(b"hello".as_slice()))]
#[case::off_alphabet_after_padding("aGVsbG8=!", Some(b"hello".as_slice()))]
#[case::padding_inside_a_quad("QU=JD", Some(b"ABC".as_slice()))]
#[case::padding_between_quads("QQ==QQ==", Some([0x41_u8, 0x04, 0x10].as_slice()))]
#[case::whitespace("\nQ U J\tD\r\n", Some(b"ABC".as_slice()))]
fn base64_decodes(#[case] input: &str, #[case] expected: Option<&[u8]>) {
    assert_eq!(decode(input).ok().as_deref(), expected);
}

#[rstest::rstest]
#[case::single_character("Q", Base64DecodeError::OneModuloFour { characters: 1 })]
#[case::padded_single_character("Q=", Base64DecodeError::OneModuloFour { characters: 1 })]
#[case::ignored_bytes_do_not_count("a?b?c?d?e", Base64DecodeError::OneModuloFour { characters: 5 })]
#[case::two_data_characters("QQ", Base64DecodeError::IncorrectPadding)]
#[case::two_data_characters_one_pad("QQ=", Base64DecodeError::IncorrectPadding)]
#[case::three_data_characters("QUJ", Base64DecodeError::IncorrectPadding)]
#[case::padding_before_two_characters("==QQ", Base64DecodeError::IncorrectPadding)]
#[case::padding_before_three_characters("=QUJ", Base64DecodeError::IncorrectPadding)]
#[case::non_ascii("\u{e9}", Base64DecodeError::NonAscii)]
#[case::non_ascii_after_data("QUJD\u{e9}", Base64DecodeError::NonAscii)]
fn base64_rejects(#[case] input: &str, #[case] expected: Base64DecodeError) {
    assert_eq!(decode(input), Err(expected));
}

#[rstest::rstest]
fn replays_the_rag_json_decoder_corpus(#[files("fuzz/corpus/rag_json_decoder/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    let ParsedJson::Object(data) = parse_json_text(&text) else {
        panic!("a corpus body must be a JSON object");
    };
    let headers = request_headers("application/json");
    let request = IngestRequest {
        headers: &headers,
        body: IngestBody::Json(&data),
    };
    if let Ok(parsed) = parse_rag_ingest_request(&request, LIMIT, &EicarTestMalwareScanner, || ID) {
        assert!(parsed.ingest_options.contains_key("vector_store"));
        assert!(parsed.file.is_none_or(|file| matches!(
            file.content_type.as_str(),
            "text/plain" | "application/pdf"
        )));
    }
}
