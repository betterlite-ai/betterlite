#![cfg(test)]

mod destination;
mod signer;
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_openai::files::records::{
    CreatePurpose, FileKind, FileObject, FileStatus, FilesPurpose,
};
use betterlite_prompts::media_metadata::ExtractedFileData;

use super::file_object::{
    convert_https_url_to_s3_uri, get_error_class, transform_create_file_response,
    uploaded_object_size,
};
use super::naming::{decode_upload_record, get_object_name};
use super::{
    CUSTOM_LLM_PROVIDER, FILE_UPLOAD_HTTP_METHOD, S3SigningError, SIGNED_HEADERS_PARAM,
    SUPPORTED_OPENAI_PARAMS, UPLOAD_CONTENT_LENGTH_PARAM, UPLOAD_URL_PARAM, validate_environment,
};
use crate::bedrock_settings::ErrorHeaders;
use crate::request_policy::{Params, RequestValue};
use support::{ID, params, rendered};

#[rstest::rstest]
#[case::managed_upload_object_name(managed_upload_object_name)]
#[case::s3_virtual_and_path_style(s3_virtual_and_path_style)]
#[case::s3_signature_service(signer::s3_signature_service)]
#[case::kms_encryption_header(signer::kms_encryption_header)]
#[case::upload_content_size(upload_content_size)]
#[case::create_file_object(create_file_object)]
#[case::upload_destination(destination::upload_destination)]
#[case::file_config_contract(file_config_contract)]
fn s3_signing_cases(#[case] case: fn()) {
    case();
}

const BATCH_NAME: &str =
    "litellm-bedrock-files-us.anthropic.claude-0-11111111-2222-4333-8444-555555555555.jsonl";
const UPLOAD_PREFIX: &str = "litellm-bedrock-files/11111111222243338444555555555555-";

fn object_name(content: &[u8], purpose: CreatePurpose, filename: Option<&str>) -> String {
    let extracted = ExtractedFileData {
        filename: filename.map(str::to_owned),
        content: content.to_vec(),
        content_type: "application/octet-stream".to_owned(),
        headers: BTreeMap::new(),
    };
    match get_object_name(&extracted, purpose, ID) {
        Ok(name) => name,
        Err(error) => format!("error: {error}"),
    }
}

fn managed_upload_object_name() {
    let first = b"{\"body\": {\"model\": \"bedrock/us.anthropic.claude:0\"}}\n{\"body\": {\"model\": \"other\"}}";
    assert_eq!(object_name(first, CreatePurpose::Batch, None), BATCH_NAME);
    let model_name = |model: &str| {
        format!("litellm-bedrock-files-{model}-11111111-2222-4333-8444-555555555555.jsonl")
    };
    assert_eq!(
        object_name(
            b"{\"body\": {\"model\": \"a:b\"}}",
            CreatePurpose::Batch,
            None
        ),
        model_name("a-b")
    );
    assert_eq!(
        object_name(b"{\"custom_id\": \"1\"}", CreatePurpose::Batch, None),
        model_name("model")
    );
    assert_eq!(
        object_name(b"{\"body\": {}}", CreatePurpose::Batch, None),
        model_name("model")
    );
    // every str.splitlines boundary ends a record, and a line Python strips to nothing is skipped
    let separated = "{\"body\": {\"model\": \"a\"}}\u{2028}{\"body\": {\"model\": \"b\"}}";
    assert_eq!(
        object_name(separated.as_bytes(), CreatePurpose::Batch, None),
        model_name("a")
    );
    let unit_separator = "\u{1f}\r\n{\"body\": {\"model\": \"q\"}}\u{b}";
    assert_eq!(
        object_name(unit_separator.as_bytes(), CreatePurpose::Batch, None),
        model_name("q")
    );
    // blank batch content and any other purpose take the upload prefix and the sanitized filename
    let upload = format!("{UPLOAD_PREFIX}a_b.jsonl");
    assert_eq!(
        object_name(b"\n  \n", CreatePurpose::Batch, Some("a b.jsonl")),
        upload
    );
    assert_eq!(
        object_name(b"", CreatePurpose::Batch, None),
        format!("{UPLOAD_PREFIX}file")
    );
    assert_eq!(
        object_name(b"\xff", CreatePurpose::Assistants, Some("../x.txt")),
        format!("{UPLOAD_PREFIX}x.txt")
    );
    assert_eq!(
        object_name(first, CreatePurpose::FineTune, None),
        format!("{UPLOAD_PREFIX}file")
    );
    // a malformed later line still fails, and only text JSON can name the model
    let utf8 = "error: the file content is not valid UTF-8";
    assert_eq!(object_name(b"\xff", CreatePurpose::Batch, None), utf8);
    let record = "error: a JSONL record of the uploaded file is not valid";
    assert_eq!(
        object_name(b"{\"a\": 1}\nnot json", CreatePurpose::Batch, None),
        record
    );
    assert_eq!(
        object_name("\u{1f}{}".as_bytes(), CreatePurpose::Batch, None),
        record
    );
    let shape = "error: a JSONL record must be an object whose body holds a string model";
    assert_eq!(
        object_name(b"{\"body\": {\"model\": 5}}", CreatePurpose::Batch, None),
        shape
    );
    assert_eq!(
        object_name(b"{\"body\": []}", CreatePurpose::Batch, None),
        shape
    );
    assert_eq!(object_name(b" [1]", CreatePurpose::Batch, None), shape);
    assert!(matches!(decode_upload_record(b" \t"), Ok(None)));
}

fn converted(url: &str) -> String {
    match convert_https_url_to_s3_uri(url) {
        Ok((uri, filename)) => format!("{uri} | {filename}"),
        Err(error) => format!("error: {error}"),
    }
}

fn s3_virtual_and_path_style() {
    let cases = [
        (
            "https://s3.us-west-2.amazonaws.com/b/k%20x?q#f",
            "s3://b/k x?q#f | k x?q#f",
        ),
        (
            "https://b.s3.us-west-2.amazonaws.com/a/b/",
            "s3://b/a/b/ | ",
        ),
        (
            "https://s3.r.amazonaws.com/b/d/%2541\nz",
            "s3://b/d/%41 | %41",
        ),
        (
            "https://my-b.s3.r.amazonaws.com/k.jsonl",
            "s3://my-b/k.jsonl | k.jsonl",
        ),
        // the regexes are case sensitive, so the fallback drops the query
        ("https://S3.us-west-2.amazonaws.com/b/k?q", "s3://b/k | k"),
        ("https://s3.cn-north-1.amazonaws.com.cn/b/k", "s3://b/k | k"),
        (
            "http://localhost:9000//bkt/dir/f.jsonl?sig=1#x",
            "s3://bkt/dir/f.jsonl | f.jsonl",
        ),
        ("https://s3.r.amazonaws.com/b/", "s3://b/ | "),
        ("https://h/b/k;v=1", "s3://b/k | k"),
        ("s3x://h/b/k;v=1", "s3://b/k;v=1 | k;v=1"),
        ("  https://h/b/\tk", "s3://b/k | k"),
        ("b/k", "s3://b/k | k"),
        (
            "https://x/onlybucket",
            "error: Unable to parse S3 URL: https://x/onlybucket",
        ),
        (
            "https://my.bucket.s3.us-west-2.amazonaws.com/k",
            "error: Unable to parse S3 URL: https://my.bucket.s3.us-west-2.amazonaws.com/k",
        ),
        (
            "https://[h/b/k",
            "error: Unable to parse S3 URL: https://[h/b/k",
        ),
        (
            "https://h]/b/k",
            "error: Unable to parse S3 URL: https://h]/b/k",
        ),
    ];
    for (url, expected) in cases {
        assert_eq!(converted(url), expected, "{url}");
    }
}

fn size(recorded: Option<RequestValue>, headers: &[(&str, &str)]) -> String {
    let mut litellm_params = Params::new();
    if let Some(value) = recorded {
        litellm_params.insert(UPLOAD_CONTENT_LENGTH_PARAM.to_owned(), value);
    }
    let response: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    match uploaded_object_size(&litellm_params, &response) {
        Ok(bytes) => bytes.to_string(),
        Err(error) => format!("error: {error}"),
    }
}

fn upload_content_size() {
    let raw = |text: &str| Some(RequestValue::Raw(text.to_owned()));
    let five = [("Content-Length", "5")];
    assert_eq!(size(raw("9"), &five), "9");
    assert_eq!(size(raw("-3"), &five), "-3");
    assert_eq!(size(raw("true"), &five), "1");
    assert_eq!(size(raw("false"), &five), "0");
    assert_eq!(size(raw("9.5"), &five), "5");
    assert_eq!(size(raw("null"), &five), "5");
    assert_eq!(size(Some(RequestValue::Text("7".to_owned())), &five), "5");
    assert_eq!(size(None, &[("content-length", "12")]), "12");
    assert_eq!(size(None, &[]), "0");
    assert_eq!(size(None, &[("Content-Length", "")]), "0");
    assert_eq!(size(None, &[("Content-Length", "-1")]), "0");
    assert_eq!(
        size(None, &[("Content-Length", "1"), ("content-length", "2")]),
        "0"
    );
    assert_eq!(size(None, &[("Content-Length", "\u{bd}")]), "0");
    let digit_like = "error: the Content-Length response header is not a whole number";
    assert_eq!(size(None, &[("Content-Length", "\u{b2}")]), digit_like);
    assert_eq!(size(None, &[("Content-Length", "1\u{b9}")]), digit_like);
    assert_eq!(size(None, &[("Content-Length", "\u{b3}")]), digit_like);
    assert_eq!(
        size(None, &[("Content-Length", "99999999999999999999")]),
        digit_like
    );
    assert_eq!(size(None, &[("X-Length", "4")]), "0");
}

fn created(litellm_params: &Params) -> Result<FileObject, S3SigningError> {
    let headers = [("Content-Length".to_owned(), "3".to_owned())];
    transform_create_file_response(litellm_params, &headers, 1_700_000_000)
}

fn create_file_object() {
    let mut litellm_params = params(&[(
        UPLOAD_URL_PARAM,
        "https://s3.us-west-2.amazonaws.com/bkt/litellm-bedrock-files/x.jsonl",
    )]);
    litellm_params.insert(
        UPLOAD_CONTENT_LENGTH_PARAM.to_owned(),
        RequestValue::Raw("42".to_owned()),
    );
    let expected = FileObject {
        id: "s3://bkt/litellm-bedrock-files/x.jsonl".to_owned(),
        bytes: 42,
        created_at: 1_700_000_000,
        filename: "x.jsonl".to_owned(),
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: Some(FileStatus::Uploaded),
        expires_at: None,
        status_details: None,
    };
    assert_eq!(created(&litellm_params).unwrap(), expected);
    let empty = FileObject {
        id: String::new(),
        bytes: 3,
        filename: String::new(),
        ..expected
    };
    assert_eq!(created(&Params::new()).unwrap(), empty);
    assert_eq!(created(&params(&[(UPLOAD_URL_PARAM, "")])).unwrap(), empty);
    let null = [(
        UPLOAD_URL_PARAM.to_owned(),
        RequestValue::Raw("null".to_owned()),
    )];
    assert_eq!(created(&null.into_iter().collect()).unwrap(), empty);
    let number = [(
        UPLOAD_URL_PARAM.to_owned(),
        RequestValue::Raw("5".to_owned()),
    )];
    let error = created(&number.into_iter().collect()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "upload_url: Input should be a valid string"
    );
    let unparsable = created(&params(&[(UPLOAD_URL_PARAM, "https://x/b")])).unwrap_err();
    assert_eq!(
        unparsable.to_string(),
        "Unable to parse S3 URL: https://x/b"
    );
    let headers = ErrorHeaders::Http(vec![("x-amzn-requestid".to_owned(), "r-1".to_owned())]);
    let class = get_error_class(503, "slow down".to_owned(), headers.clone()).unwrap();
    assert_eq!(class.error.status_code, 503);
    assert_eq!(class.error.message, "slow down");
    assert_eq!(class.headers, Some(headers));
}

fn file_config_contract() {
    assert_eq!(CUSTOM_LLM_PROVIDER, "bedrock");
    assert_eq!(FILE_UPLOAD_HTTP_METHOD, "PUT");
    let no_params: [&str; 0] = [];
    assert_eq!(SUPPORTED_OPENAI_PARAMS, no_params);
    let headers = params(&[("a", "1"), ("c", "4")]);
    let mut litellm_params = params(&[("keep", "x")]);
    litellm_params.insert(
        SIGNED_HEADERS_PARAM.to_owned(),
        RequestValue::Mapping(params(&[("a", "2"), ("b", "3")])),
    );
    let merged = validate_environment(&headers, &mut litellm_params);
    assert_eq!(
        rendered(&merged),
        [("a", "2"), ("c", "4"), ("b", "3")]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
    assert_eq!(
        rendered(&litellm_params),
        [("keep".to_owned(), "x".to_owned())]
    );
    // a non-mapping value is consumed without touching the headers
    litellm_params.insert(
        SIGNED_HEADERS_PARAM.to_owned(),
        RequestValue::Text("x".to_owned()),
    );
    assert_eq!(
        rendered(&validate_environment(&headers, &mut litellm_params)),
        rendered(&headers)
    );
    assert!(!litellm_params.contains_key(SIGNED_HEADERS_PARAM));
    assert_eq!(
        rendered(&validate_environment(&headers, &mut litellm_params)),
        rendered(&headers)
    );
}

#[test]
fn replays_the_bedrock_services_s3_signing_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_s3_signing");
    let mut seeds = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus directory exists") {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let expected = name.starts_with("valid") || name.starts_with("boundary-valid");
        assert_eq!(
            decode_upload_record(&bytes).is_ok(),
            expected,
            "seed {name}"
        );
        seeds = seeds.saturating_add(1);
    }
    assert!(seeds >= 6, "the corpus holds only {seeds} seeds");
}
