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

fn row_with_value(value: &[u8]) -> Vec<u8> {
    let mut row = b"{\"custom_id\":".to_vec();
    row.extend(value);
    row.extend(b",\"method\":1,\"url\":1,\"body\":1}");
    row
}

fn invalid_first_line() -> Option<BatchValidationFailure> {
    Some(BatchValidationFailure::InvalidJsonLine { line_number: 1 })
}

#[rstest]
#[case::every_escape(br#""\"\\\/\b\f\n\r\t\u0041\u00e9\ud800""#)]
#[case::negative_infinity(b"-Infinity")]
#[case::infinity(b"Infinity")]
#[case::not_a_number(b"NaN")]
#[case::zero(b"0")]
#[case::negative_zero(b"-0")]
#[case::negative(b"-12")]
#[case::fraction(b"1.50")]
#[case::exponent(b"1e5")]
#[case::signed_exponent(b"1E+5")]
#[case::negative_exponent(b"2.5e-3")]
#[case::nested_array(b"[1, [2, []], {}]")]
#[case::nested_object(br#"{"a": [true, false, null], "b": {}}"#)]
#[case::spaced_array(b"[ 1 , 2 ]")]
fn json_byte_compatibility_accepts_values(#[case] value: &[u8]) {
    assert!(batch_failure(&row_with_value(value), Some("x.jsonl")).is_none());
}

#[rstest]
#[case::double_minus(b"--1")]
#[case::lone_minus(b"-")]
#[case::minus_word(b"-Inf")]
#[case::plus_sign(b"+1")]
#[case::leading_zero(b"01")]
#[case::bare_point(b"1.")]
#[case::bare_exponent(b"1e")]
#[case::signed_bare_exponent(b"1e+")]
#[case::lowercase_nan(b"Nan")]
#[case::short_true(b"tr")]
#[case::short_null(b"nul")]
#[case::trailing_comma(b"[1,]")]
#[case::missing_comma(b"[1 2]")]
#[case::unclosed_array(b"[1")]
#[case::member_without_value(br#"{"a"}"#)]
#[case::object_trailing_comma(br#"{"a":1,}"#)]
#[case::unterminated_string(br#""ab"#)]
#[case::bad_escape(br#""\x""#)]
#[case::short_unicode_escape(br#""\u12""#)]
#[case::non_hex_unicode_escape(br#""\u12G4""#)]
#[case::signed_unicode_escape(br#""\u+123""#)]
#[case::control_character(b"\"a\x01\"")]
fn json_byte_compatibility_rejects_values(#[case] value: &[u8]) {
    let failure = batch_failure(&row_with_value(value), Some("x.jsonl"));
    assert_eq!(failure, invalid_first_line());
}

#[rstest]
#[case::trailing_text(b"{}x")]
#[case::second_object(b"{}{}")]
#[case::unclosed_object(b"{")]
#[case::bare_word(b"abc")]
fn json_byte_compatibility_rejects_broken_lines(#[case] source: &[u8]) {
    assert_eq!(batch_failure(source, Some("x.jsonl")), invalid_first_line());
}

fn nested_arrays(levels: usize) -> Vec<u8> {
    let mut value = b"[".repeat(levels);
    value.extend(b"]".repeat(levels));
    value
}

fn nested_objects(levels: usize) -> Vec<u8> {
    let mut value = br#"{"a":"#.repeat(levels);
    value.extend(b"1");
    value.extend(b"}".repeat(levels));
    value
}

#[rstest]
#[case::at_the_cap(128, None)]
#[case::over_the_cap(129, invalid_first_line())]
fn json_byte_compatibility_depth_cap_for_arrays(
    #[case] levels: usize,
    #[case] expected: Option<BatchValidationFailure>,
) {
    let row = row_with_value(&nested_arrays(levels));
    assert_eq!(batch_failure(&row, Some("x.jsonl")), expected);
}

#[rstest]
#[case::at_the_cap(127, None)]
#[case::over_the_cap(128, invalid_first_line())]
fn json_byte_compatibility_depth_cap_for_objects(
    #[case] levels: usize,
    #[case] expected: Option<BatchValidationFailure>,
) {
    let row = row_with_value(&nested_objects(levels));
    assert_eq!(batch_failure(&row, Some("x.jsonl")), expected);
}

#[rstest]
#[case::at_the_cap(129, Some(BatchValidationFailure::LineNotObject { line_number: 1 }))]
#[case::over_the_cap(130, invalid_first_line())]
fn json_byte_compatibility_depth_cap_at_the_top(
    #[case] levels: usize,
    #[case] expected: Option<BatchValidationFailure>,
) {
    assert_eq!(
        batch_failure(&nested_arrays(levels), Some("x.jsonl")),
        expected
    );
}

#[test]
fn json_byte_compatibility_long_arrays_do_not_recurse() {
    let mut value = b"[1".to_vec();
    value.extend(b",1".repeat(400_000));
    value.extend(b"]");
    assert!(batch_failure(&row_with_value(&value), Some("x.jsonl")).is_none());
}

#[rstest]
#[case::escaped_underscore(br#"{"custom\u005fid":1,"method":1,"url":1,"body":1}"#)]
#[case::escaped_first_letter(br#"{"\u0063ustom_id":1,"method":1,"url":1,"body":1}"#)]
#[case::escaped_inner_letter(br#"{"custom_id":1,"m\u0065thod":1,"url":1,"body":1}"#)]
fn json_byte_compatibility_escaped_keys_count(#[case] source: &[u8]) {
    assert!(batch_failure(source, Some("x.jsonl")).is_none());
}

#[rstest]
#[case::raw_non_ascii_first(r#"{"é\u0063ustom_id":1,"method":1,"url":1,"body":1}"#)]
#[case::escaped_non_ascii_first(r#"{"\u00e9\u0063ustom_id":1,"method":1,"url":1,"body":1}"#)]
#[case::escaped_non_ascii_last(r#"{"custom_id\u00e9":1,"method":1,"url":1,"body":1}"#)]
#[case::escape_at_the_ascii_edge(r#"{"custom_id\u0080":1,"method":1,"url":1,"body":1}"#)]
fn json_byte_compatibility_non_ascii_keys_never_match(#[case] source: &str) {
    let failure = batch_failure(source.as_bytes(), Some("x.jsonl"));
    assert_eq!(
        failure,
        Some(BatchValidationFailure::MissingLineKey {
            line_number: 1,
            key: "custom_id",
            line_shape: LineShape::OpenAi,
        })
    );
}

#[test]
fn json_byte_compatibility_nested_keys_do_not_count() {
    let source = br#"{"a":{"custom_id":1,"method":1,"url":1,"body":1}}"#;
    assert_eq!(
        batch_failure(source, Some("x.jsonl")),
        Some(BatchValidationFailure::MissingLineKey {
            line_number: 1,
            key: "custom_id",
            line_shape: LineShape::OpenAi,
        })
    );
}

#[rstest]
#[case::lf(b"\n{", 2)]
#[case::crlf(b"\r\n{", 2)]
#[case::bare_cr(b"\r{", 2)]
#[case::cr_cr(b"\r\r{", 3)]
#[case::lf_cr(b"\n\r{", 3)]
#[case::vertical_tab_is_blank(b"\x0b\n{", 2)]
#[case::form_feed_is_blank(b"\x0c\n{", 2)]
#[case::tab_is_blank(b"\t\n{", 2)]
fn batch_line_failures_number_the_physical_lines(#[case] source: &[u8], #[case] line: usize) {
    assert_eq!(
        batch_failure(source, Some("x.jsonl")),
        Some(BatchValidationFailure::InvalidJsonLine { line_number: line })
    );
}

#[rstest]
#[case::utf16_le(&[0xFF, 0xFE], false)]
#[case::utf16_be(&[0xFE, 0xFF], true)]
fn json_byte_compatibility_utf16_boms(#[case] bom: &[u8], #[case] big_endian: bool) {
    let mut line = bom.to_vec();
    for unit in OPENAI_ROW {
        match big_endian {
            true => line.extend([0, *unit]),
            false => line.extend([*unit, 0]),
        }
    }
    assert!(batch_failure(&line, Some("x.jsonl")).is_none());
}

#[rstest]
#[case::utf16_le_sniffed(b"{\0}\0")]
#[case::utf16_be_sniffed(b"\0{\0}")]
#[case::utf32_le_sniffed(b"{\0\0\0}\0\0\0")]
#[case::utf32_be_sniffed(b"\0\0\0{\0\0\0}")]
fn json_byte_compatibility_sniffed_encodings(#[case] line: &[u8]) {
    let failure = batch_failure(line, Some("x.jsonl")).unwrap();
    assert!(matches!(
        failure,
        BatchValidationFailure::MissingLineKey {
            key: "custom_id",
            ..
        }
    ));
}

#[rstest]
#[case::utf16_le_two_bytes(b"1\0")]
#[case::utf16_be_two_bytes(b"\x001")]
fn json_byte_compatibility_two_byte_encodings(#[case] line: &[u8]) {
    assert_eq!(
        batch_failure(line, Some("x.jsonl")),
        Some(BatchValidationFailure::LineNotObject { line_number: 1 })
    );
}

#[rstest]
#[case::utf16_le_pair(b"{\0")]
#[case::utf16_be_pair(b"\0{")]
#[case::odd_utf16_length(b"\xff\xfe{\0}\0 ")]
#[case::odd_utf32_length(b"\xff\xfe\0\0{\0\0\0}\0\0\0 ")]
#[case::bom_only(b"\xef\xbb\xbf")]
fn json_byte_compatibility_rejects_truncated_encodings(#[case] line: &[u8]) {
    assert_eq!(batch_failure(line, Some("x.jsonl")), invalid_first_line());
}

#[rstest]
#[case::overlong_two_byte(b"\"\xc0\x80\"")]
#[case::lone_continuation(b"\"\x80\"")]
#[case::truncated_three_byte(b"\"\xe2\x82\"")]
#[case::bad_continuation(b"\"\xe2\x28\xa1\"")]
#[case::overlong_three_byte(b"\"\xe0\x80\x80\"")]
#[case::above_unicode(b"\"\xf4\x90\x80\x80\"")]
#[case::overlong_four_byte(b"\"\xf0\x80\x80\x80\"")]
#[case::five_byte_lead(b"\"\xf8\x88\x80\x80\x80\"")]
fn json_byte_compatibility_rejects_malformed_utf8(#[case] value: &[u8]) {
    let failure = batch_failure(&row_with_value(value), Some("x.jsonl"));
    assert_eq!(failure, invalid_first_line());
}

#[rstest]
#[case::two_byte(b"\"\xc2\x80\"")]
#[case::three_byte(b"\"\xe2\x82\xac\"")]
#[case::three_byte_low_lead(b"\"\xe0\xa0\x80\"")]
#[case::encoded_surrogate(b"\"\xed\xa0\x80\"")]
#[case::four_byte(b"\"\xf0\x9f\x98\x80\"")]
#[case::four_byte_middle_lead(b"\"\xf1\x80\x80\x80\"")]
#[case::highest_scalar(b"\"\xf4\x8f\xbf\xbf\"")]
fn json_byte_compatibility_accepts_wellformed_utf8(#[case] value: &[u8]) {
    assert!(batch_failure(&row_with_value(value), Some("x.jsonl")).is_none());
}

#[rstest]
#[case::highest_scalar(0x0010_FFFF, None)]
#[case::above_unicode(0x0011_0000, invalid_first_line())]
fn json_byte_compatibility_utf32_limits(
    #[case] scalar: u32,
    #[case] expected: Option<BatchValidationFailure>,
) {
    let mut line = vec![0x00, 0x00, 0xFE, 0xFF];
    for unit in b"{\"custom_id\":\"" {
        line.extend(u32::from(*unit).to_be_bytes());
    }
    line.extend(scalar.to_be_bytes());
    for unit in b"\",\"method\":1,\"url\":1,\"body\":1}" {
        line.extend(u32::from(*unit).to_be_bytes());
    }
    assert_eq!(batch_failure(&line, Some("x.jsonl")), expected);
}

#[rstest]
#[case::latin1_range(0x00E9)]
#[case::above_latin1(0x20AC)]
#[case::highest_byte(0x00FF)]
#[case::first_non_ascii(0x0080)]
#[case::low_byte_is_a_letter(0x0141)]
fn json_byte_compatibility_utf16_non_ascii_values(#[case] unit: u16) {
    let mut line = vec![0xFF, 0xFE];
    for byte in b"{\"custom_id\":\"" {
        line.extend(u16::from(*byte).to_le_bytes());
    }
    line.extend(unit.to_le_bytes());
    for byte in b"\",\"method\":1,\"url\":1,\"body\":1}" {
        line.extend(u16::from(*byte).to_le_bytes());
    }
    assert!(batch_failure(&line, Some("x.jsonl")).is_none());
}

#[test]
fn extension_policy_missing_list_never_fails() {
    assert!(check_extension(None, None, ExtensionRule::Allowed).is_none());
    assert!(check_extension(Some("x"), None, ExtensionRule::Allowed).is_none());
    assert!(check_extension(Some(".env"), None, ExtensionRule::Allowed).is_none());
    assert!(check_extension(None, None, ExtensionRule::Blocked).is_none());
}

#[test]
fn extension_policy_empty_entries_never_match() {
    let blank = names(&[".pdf", ""]);
    assert!(check_extension(Some("x"), Some(&blank), ExtensionRule::Blocked).is_none());
    assert!(check_extension(None, Some(&blank), ExtensionRule::Blocked).is_none());
    assert!(check_extension(Some("x.png"), Some(&blank), ExtensionRule::Blocked).is_none());
    assert_eq!(
        check_extension(Some("x"), Some(&blank), ExtensionRule::Allowed),
        Some(UploadValidationFailure::ExtensionNotAllowed {
            extension: String::new(),
        })
    );
}

#[rstest]
#[case::first_entry(&[".pdf", ".txt"])]
#[case::last_entry(&[".txt", ".pdf"])]
#[case::middle_entry(&[".txt", ".pdf", ".md"])]
fn extension_policy_finds_any_listed_entry(#[case] list: &[&str]) {
    let list = names(list);
    assert!(check_extension(Some("a.pdf"), Some(&list), ExtensionRule::Allowed).is_none());
    assert_eq!(
        check_extension(Some("a.pdf"), Some(&list), ExtensionRule::Blocked),
        Some(UploadValidationFailure::BlockedExtension {
            extension: ".pdf".to_owned(),
        })
    );
}

#[rstest]
#[case::plain("report.pdf", ".pdf")]
#[case::upper("REPORT.PDF", ".pdf")]
#[case::last_suffix("archive.tar.gz", ".gz")]
#[case::directory("dir/report.pdf", ".pdf")]
#[case::backslash_directory(r"dir\report.pdf", ".pdf")]
#[case::dotted_directory("a.b/report", "")]
#[case::hidden(".env", "")]
#[case::trailing_dot("report.", "")]
#[case::only_dot("a.b.", "")]
fn extension_policy_suffix_rules(#[case] filename: &str, #[case] extension: &str) {
    let failure = check_extension(Some(filename), Some(&[]), ExtensionRule::Allowed);
    assert_eq!(
        failure,
        Some(UploadValidationFailure::ExtensionNotAllowed {
            extension: extension.to_owned(),
        })
    );
}

#[rstest]
#[case::exactly_one_mib(1_048_576, 1, false)]
#[case::one_byte_over(1_048_577, 1, true)]
#[case::two_mib_boundary(2_097_152, 2, false)]
#[case::two_mib_over(2_097_153, 2, true)]
#[case::zero_cap_is_off(10, 0, false)]
fn upload_size_boundaries_per_megabyte(
    #[case] length: usize,
    #[case] limit: i64,
    #[case] rejected: bool,
) {
    let data = vec![b'x'; length];
    assert_eq!(upload_size(&data, Some(limit)).is_some(), rejected);
}

#[rstest]
#[case::exact_tenth(1_048_577, "1.0")]
#[case::rounds_up(1_153_433, "1.1")]
#[case::below_half_tenth(1_100_000, "1.0")]
#[case::above_half_tenth(1_105_000, "1.1")]
#[case::tie_to_even_down(1_310_720, "1.2")]
#[case::tie_to_even_up(1_835_008, "1.8")]
#[case::large(10_485_760, "10.0")]
fn upload_size_boundaries_message_rounding(#[case] length: usize, #[case] shown: &str) {
    let data = vec![b'x'; length];
    let failure = upload_size(&data, Some(1)).unwrap();
    assert_eq!(
        failure.fields().message,
        format!(
            "Uploaded file exceeds the configured max_file_size_mb of 1 MB (read stopped at {shown} MB). The file was not forwarded to the provider."
        )
    );
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
