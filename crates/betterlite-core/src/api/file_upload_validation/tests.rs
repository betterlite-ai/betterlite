#![cfg(test)]

use std::fs;
use std::io::Cursor;
use std::path::Path;

use rstest::rstest;

use super::batch::{
    BatchValidationFailure, check_batch_file_upload, decode_file_upload_validation,
};
use super::failure::exception;
use super::line_shape::LineShape;
use super::source::FileBody;
use super::upload::{
    ExtensionRule, UploadValidationFailure, check_extension, check_unsafe_filename,
    check_upload_file_size,
};
use crate::api::api_boundaries::ApiLimits;

const OPENAI_ROW: &[u8] = b"{\"custom_id\":1,\"method\":\"m\",\"url\":\"u\",\"body\":{}}";
const OPENAI_ROW_NEWLINE: &[u8] = b"{\"custom_id\":1,\"method\":\"m\",\"url\":\"u\",\"body\":{}}\n";

fn limits() -> ApiLimits {
    ApiLimits::default()
}

fn batch_failure(source: &[u8], filename: Option<&str>) -> Option<BatchValidationFailure> {
    let mut body = FileBody::Bytes(source);
    check_batch_file_upload(filename, &mut body, None, LineShape::OpenAi, limits()).unwrap()
}

fn padded(size: usize) -> Vec<u8> {
    let mut bytes = OPENAI_ROW.to_vec();
    bytes.resize(size, b' ');
    bytes
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::dot_env(Some(".env"))]
#[case::double_dot(Some("report..pdf"))]
#[case::versioned(Some("report.v2.pdf"))]
#[case::directory(Some("dir/file.pdf"))]
fn unsafe_filename_components_accept(#[case] filename: Option<&str>) {
    assert!(check_unsafe_filename(filename).is_none());
}

#[rstest]
#[case::parent("../file")]
#[case::inner_parent("dir/../file")]
#[case::backslash(r"dir\..\file")]
#[case::null_byte("a\0b")]
fn unsafe_filename_components_reject(#[case] filename: &str) {
    let failure = check_unsafe_filename(Some(filename)).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(
        fields.message,
        format!(
            "Filename '{filename}' is not allowed: directory traversal sequences are not permitted in uploaded file names. The file was not forwarded to the provider."
        )
    );
    assert_eq!(fields.status, 400);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("file".to_owned()));
    assert_eq!(exception.code, "400");
}

#[test]
fn extension_policy_allowlist_and_blocklist() {
    let pdf = names(&[".pdf"]);
    let upper = names(&[".PDF"]);
    assert!(check_extension(Some("REPORT.PDF"), Some(&pdf), ExtensionRule::Allowed).is_none());
    assert!(check_extension(Some("REPORT.PDF"), Some(&upper), ExtensionRule::Allowed).is_none());
    let failure =
        check_extension(Some("report.pdf"), Some(&upper), ExtensionRule::Blocked).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(
        fields.message,
        "File extension '.pdf' is blocked by this proxy's blocked_file_extensions setting. The file was not forwarded to the provider."
    );
    assert_eq!(fields.status, 400);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("file".to_owned()));
    assert_eq!(exception.code, "400");
    assert!(check_extension(Some("report.pdf"), Some(&[]), ExtensionRule::Blocked).is_none());
    assert!(check_extension(Some("report.pdf"), None, ExtensionRule::Allowed).is_none());
    assert!(check_extension(Some("report.pdf"), None, ExtensionRule::Blocked).is_none());
}

#[rstest]
#[case::plain_allowed_miss(&["pdf"], Some("x.pdf"), false)]
#[case::empty_allowlist(&[], Some("x.pdf"), false)]
#[case::empty_allowlist_bare_name(&[], Some("x"), true)]
#[case::missing_name(&[".pdf"], None, true)]
#[case::dot_env(&[".pdf"], Some(".env"), true)]
#[case::traversal_without_suffix(&[".pdf"], Some("../dir"), true)]
#[case::discarded_null(&[".pdf"], Some("a\0b"), true)]
fn extension_policy_allow_failures(
    #[case] allow: &[&str],
    #[case] filename: Option<&str>,
    #[case] missing_extension: bool,
) {
    let allow = names(allow);
    let failure = check_extension(filename, Some(&allow), ExtensionRule::Allowed).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(fields.status, 400);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("file".to_owned()));
    assert_eq!(exception.code, "400");
    let subject = match missing_extension {
        true => "A file without an extension".to_owned(),
        false => "File extension '.pdf'".to_owned(),
    };
    assert_eq!(
        fields.message,
        format!(
            "{subject} is not in this proxy's allowed_file_extensions setting. The file was not forwarded to the provider."
        )
    );
}

fn upload_size(source: &[u8], limit: Option<i64>) -> Option<UploadValidationFailure> {
    let mut body = FileBody::Bytes(source);
    check_upload_file_size(&mut body, limit).unwrap()
}

#[rstest]
#[case::at_the_cap(1_048_576, Some(1))]
#[case::empty_with_no_cap(0, None)]
#[case::empty_with_zero_cap(0, Some(0))]
#[case::small_with_negative_cap(10, Some(-1))]
#[case::huge_cap(10, Some(i64::MAX))]
fn upload_size_boundaries_accept(#[case] length: usize, #[case] limit: Option<i64>) {
    let data = vec![b'x'; length];
    assert!(upload_size(&data, limit).is_none());
}

#[test]
fn upload_size_boundaries_reject() {
    let data = vec![b'x'; 1_048_577];
    let failure = upload_size(&data, Some(1)).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(
        fields.message,
        "Uploaded file exceeds the configured max_file_size_mb of 1 MB (read stopped at 1.0 MB). The file was not forwarded to the provider."
    );
    assert_eq!(fields.status, 413);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("file".to_owned()));
    assert_eq!(exception.code, "413");
}

#[test]
fn upload_size_boundaries_restore_a_seekable_position() {
    let mut cursor = Cursor::new(vec![b'x'; 1_048_577]);
    cursor.set_position(7);
    let mut body = FileBody::Seekable(&mut cursor);
    let failure = check_upload_file_size(&mut body, Some(1)).unwrap();
    drop(body);
    assert!(failure.is_some());
    assert_eq!(cursor.position(), 7);

    let mut small = Cursor::new(b"abcdefgh".to_vec());
    small.set_position(7);
    let mut body = FileBody::Seekable(&mut small);
    let failure = check_upload_file_size(&mut body, Some(1)).unwrap();
    drop(body);
    assert!(failure.is_none());
    assert_eq!(small.position(), 7);
}

#[test]
fn batch_size_and_extension_prefers_the_extension() {
    let oversized = vec![b'x'; 2_000_000];
    assert_eq!(
        batch_failure(&oversized, Some("x.txt")),
        Some(BatchValidationFailure::WrongExtension {
            filename: "x.txt".to_owned(),
        })
    );
    assert!(batch_failure(OPENAI_ROW, Some("x.JSONL")).is_none());
}

#[test]
fn batch_size_and_extension_absent_name() {
    let failure = batch_failure(b"x", None).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(
        fields.message,
        "Invalid file format for Batch API: ''. Batch input files must be .jsonl files. The file was not forwarded to the provider."
    );
    assert_eq!(fields.status, 400);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("file".to_owned()));
    assert_eq!(exception.code, "400");
}

#[test]
fn batch_size_and_extension_cap() {
    let at_cap = padded(1_048_576);
    let mut body = FileBody::Bytes(&at_cap);
    assert!(
        check_batch_file_upload(
            Some("x.jsonl"),
            &mut body,
            Some(1),
            LineShape::OpenAi,
            limits()
        )
        .unwrap()
        .is_none()
    );

    let over_cap = padded(1_048_577);
    let mut body = FileBody::Bytes(&over_cap);
    let failure = check_batch_file_upload(
        Some("x.jsonl"),
        &mut body,
        Some(1),
        LineShape::OpenAi,
        limits(),
    )
    .unwrap()
    .unwrap();
    let fields = failure.fields();
    assert_eq!(
        fields.message,
        "Batch input file is 1.0 MB, which exceeds the configured max_batch_file_size_mb of 1 MB. The file was not forwarded to the provider."
    );
    assert_eq!(fields.status, 413);
}

#[rstest]
#[case::no_cap(None)]
#[case::zero_cap(Some(0))]
#[case::negative_cap(Some(-1))]
#[case::huge_cap(Some(i64::MAX))]
fn batch_size_and_extension_caps_disabled(#[case] limit: Option<i64>) {
    let mut cursor = Cursor::new(vec![b'\n'; 1_048_577]);
    let mut body = FileBody::Seekable(&mut cursor);
    let failure = check_batch_file_upload(
        Some("x.jsonl"),
        &mut body,
        limit,
        LineShape::OpenAi,
        limits(),
    )
    .unwrap()
    .unwrap();
    drop(body);
    assert_eq!(failure, BatchValidationFailure::Empty);
    assert_eq!(cursor.position(), 0);
}

#[rstest]
#[case::empty(
    b"",
    None,
    "Batch input file has no request lines. The file was not forwarded to the provider."
)]
#[case::blank_lines(
    b"\n \r\n",
    None,
    "Batch input file has no request lines. The file was not forwarded to the provider."
)]
#[case::malformed_after_blank(
    b"\n{",
    Some(2),
    "Batch input file line 2 is not valid JSON. The file was not forwarded to the provider."
)]
#[case::invalid_utf8_after_blank(
    b"\n\xff",
    Some(2),
    "Batch input file line 2 is not valid JSON. The file was not forwarded to the provider."
)]
#[case::array_line(
    b"\n[]",
    Some(2),
    "Batch input file line 2 must be a JSON object. The file was not forwarded to the provider."
)]
#[case::first_line_wins(
    b"{\n{\n{}",
    Some(1),
    "Batch input file line 1 is not valid JSON. The file was not forwarded to the provider."
)]
fn batch_line_failures(
    #[case] source: &[u8],
    #[case] line_number: Option<usize>,
    #[case] message: &str,
) {
    let failure = batch_failure(source, Some("x.jsonl")).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(fields.message, message);
    assert_eq!(fields.status, 400);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some("file".to_owned()));
    assert_eq!(exception.code, "400");
    let reported = match failure {
        BatchValidationFailure::InvalidJsonLine { line_number }
        | BatchValidationFailure::LineNotObject { line_number } => Some(line_number),
        BatchValidationFailure::TooLarge { .. }
        | BatchValidationFailure::WrongExtension { .. }
        | BatchValidationFailure::Empty
        | BatchValidationFailure::MissingLineKey { .. } => None,
    };
    assert_eq!(reported, line_number);
}

#[test]
fn batch_line_failures_rewind_a_seekable_scan() {
    let mut cursor = Cursor::new(b"\n{\n".to_vec());
    cursor.set_position(3);
    let mut body = FileBody::Seekable(&mut cursor);
    let failure = check_batch_file_upload(
        Some("x.jsonl"),
        &mut body,
        None,
        LineShape::OpenAi,
        limits(),
    )
    .unwrap();
    drop(body);
    assert!(failure.is_some());
    assert_eq!(cursor.position(), 0);

    let mut cursor = Cursor::new(OPENAI_ROW_NEWLINE.to_vec());
    let mut body = FileBody::Seekable(&mut cursor);
    let failure = check_batch_file_upload(
        Some("x.jsonl"),
        &mut body,
        None,
        LineShape::OpenAi,
        limits(),
    )
    .unwrap();
    drop(body);
    assert!(failure.is_none());
    assert_eq!(cursor.position(), 0);
}

#[rstest]
#[case::empty_object(b"{}", "custom_id")]
#[case::one_key(b"{\"custom_id\":1}", "method")]
#[case::two_keys(b"{\"custom_id\":1,\"method\":\"m\"}", "url")]
#[case::three_keys(b"{\"custom_id\":1,\"method\":\"m\",\"url\":\"u\"}", "body")]
fn required_batch_keys(#[case] source: &[u8], #[case] key: &str) {
    let failure = batch_failure(source, Some("x.jsonl")).unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(
        fields.message,
        format!(
            "Missing required parameter: '{key}' (batch input file line 1). Each line must be a JSON object with keys custom_id, method, url, body. The file was not forwarded to the provider."
        )
    );
    assert_eq!(fields.status, 400);
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param, Some(key.to_owned()));
    assert_eq!(exception.code, "400");
}

#[test]
fn required_batch_keys_accept_null_values() {
    let source = b"{\"custom_id\":null,\"method\":null,\"url\":null,\"body\":null}";
    assert!(batch_failure(source, Some("x.jsonl")).is_none());
}

#[test]
fn required_batch_keys_vertex_shape() {
    assert!(batch_failure(b"{\"request\":null}", Some("x.jsonl")).is_none());
    let mut body = FileBody::Bytes(b"{}");
    let failure = check_batch_file_upload(
        Some("x.jsonl"),
        &mut body,
        None,
        LineShape::VertexPassthrough,
        limits(),
    )
    .unwrap()
    .unwrap();
    let fields = failure.fields();
    let exception = exception(fields.clone());
    assert_eq!(
        fields.message,
        "Missing required parameter: 'request' (batch input file line 1). A passthrough upload takes native Vertex batch rows, so each line must be a JSON object with a request key. The file was not forwarded to the provider."
    );
    assert_eq!(exception.param, Some("request".to_owned()));
}

#[test]
fn json_byte_compatibility_bom_encodings() {
    let mut utf8 = vec![0xEF, 0xBB, 0xBF];
    utf8.extend(OPENAI_ROW);
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(OPENAI_ROW.iter().flat_map(|byte| [*byte, 0]));
    let mut utf32 = vec![0x00, 0x00, 0xFE, 0xFF];
    utf32.extend(OPENAI_ROW.iter().flat_map(|byte| [0, 0, 0, *byte]));
    assert!(batch_failure(&utf8, Some("x.jsonl")).is_none());
    assert!(batch_failure(&utf16, Some("x.jsonl")).is_none());
    assert!(batch_failure(&utf32, Some("x.jsonl")).is_none());
}

#[test]
fn json_byte_compatibility_surrogates_and_duplicates() {
    let surrogate = b"{\"custom_id\":\"\\ud800\",\"method\":\"m\",\"url\":\"u\",\"body\":{}}";
    assert!(batch_failure(surrogate, Some("x.jsonl")).is_none());
    let duplicates =
        b"{\"custom_id\":1,\"custom_id\":2,\"method\":null,\"url\":null,\"body\":null}";
    assert!(batch_failure(duplicates, Some("x.jsonl")).is_none());
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/file_upload_validation");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .map(|bytes| {
            drop(decode_file_upload_validation(
                &bytes,
                LineShape::OpenAi,
                limits(),
            ))
        })
        .count();
    assert!(replayed >= 8);
}
