#![cfg(test)]

mod destination;
mod naming;
mod signer;
mod support;
mod urls;

use std::fs;
use std::path::PathBuf;

use betterlite_openai::files::records::{FileKind, FileObject, FileStatus, FilesPurpose};

use super::file_object::{get_error_class, transform_create_file_response, uploaded_object_size};
use super::naming::decode_upload_record;
use super::{
    CUSTOM_LLM_PROVIDER, FILE_UPLOAD_HTTP_METHOD, S3SigningError, SIGNED_HEADERS_PARAM,
    SUPPORTED_OPENAI_PARAMS, UPLOAD_CONTENT_LENGTH_PARAM, UPLOAD_URL_PARAM, validate_environment,
};
use crate::bedrock_settings::ErrorHeaders;
use crate::request_policy::{Params, RequestValue};
use support::{params, rendered};

#[rstest::rstest]
#[case::managed_upload_object_name(naming::managed_upload_object_name)]
#[case::s3_virtual_and_path_style(urls::s3_virtual_and_path_style)]
#[case::s3_signature_service(signer::s3_signature_service)]
#[case::kms_encryption_header(signer::kms_encryption_header)]
#[case::upload_content_size(upload_content_size)]
#[case::create_file_object(create_file_object)]
#[case::upload_destination(destination::upload_destination)]
#[case::file_config_contract(file_config_contract)]
fn s3_signing_cases(#[case] case: fn()) {
    case();
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
    let repeated = [("Content-Length", "1"), ("content-length", "2")];
    assert_eq!(size(None, &repeated), "0");
    assert_eq!(size(None, &[("Content-Length", "\u{bd}")]), "0");
    assert_eq!(size(None, &[("X-Length", "4")]), "0");
    let digit_like = "error: the Content-Length response header is not a whole number";
    for length in ["\u{b2}", "1\u{b9}", "\u{b3}", "99999999999999999999"] {
        assert_eq!(size(None, &[("Content-Length", length)]), digit_like);
    }
}

fn created(litellm_params: &Params) -> Result<FileObject, S3SigningError> {
    let headers = [("Content-Length".to_owned(), "3".to_owned())];
    transform_create_file_response(litellm_params, &headers, 1_700_000_000)
}

fn create_file_object() {
    file_from_upload_url();
    unusable_upload_url();
    error_class_delegation();
}

fn file_from_upload_url() {
    let url = "https://s3.us-west-2.amazonaws.com/bkt/litellm-bedrock-files/x.jsonl";
    let mut litellm_params = params(&[(UPLOAD_URL_PARAM, url)]);
    let recorded = RequestValue::Raw("42".to_owned());
    litellm_params.insert(UPLOAD_CONTENT_LENGTH_PARAM.to_owned(), recorded);
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
    // a missing, empty or null upload_url leaves the id and filename empty
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
}

fn unusable_upload_url() {
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
}

fn error_class_delegation() {
    let headers = ErrorHeaders::Http(vec![("x-amzn-requestid".to_owned(), "r-1".to_owned())]);
    let class = get_error_class(503, "slow down".to_owned(), headers.clone()).unwrap();
    assert_eq!(class.error.status_code, 503);
    assert_eq!(class.error.message, "slow down");
    assert_eq!(class.headers, Some(headers));
    assert_eq!(
        class.response_headers,
        [("x-amzn-requestid".to_owned(), "r-1".to_owned())]
    );
}

fn file_config_contract() {
    assert_eq!(CUSTOM_LLM_PROVIDER, "bedrock");
    assert_eq!(FILE_UPLOAD_HTTP_METHOD, "PUT");
    let no_params: [&str; 0] = [];
    assert_eq!(SUPPORTED_OPENAI_PARAMS, no_params);
    let headers = params(&[("a", "1"), ("c", "4")]);
    let mut litellm_params = params(&[("keep", "x")]);
    let signed = RequestValue::Mapping(params(&[("a", "2"), ("b", "3")]));
    litellm_params.insert(SIGNED_HEADERS_PARAM.to_owned(), signed);
    // the signed values win over the input headers, and the handoff leaves litellm_params
    let merged = validate_environment(&headers, &mut litellm_params);
    assert_eq!(
        rendered(&merged),
        rendered(&params(&[("a", "2"), ("c", "4"), ("b", "3")]))
    );
    assert_eq!(
        rendered(&litellm_params),
        rendered(&params(&[("keep", "x")]))
    );
    // a non-mapping value is consumed without touching the headers
    let text = RequestValue::Text("x".to_owned());
    litellm_params.insert(SIGNED_HEADERS_PARAM.to_owned(), text);
    let unsigned = validate_environment(&headers, &mut litellm_params);
    assert_eq!(rendered(&unsigned), rendered(&headers));
    assert!(!litellm_params.contains_key(SIGNED_HEADERS_PARAM));
    let absent = validate_environment(&headers, &mut litellm_params);
    assert_eq!(rendered(&absent), rendered(&headers));
}

#[rstest::rstest]
fn replays_the_bedrock_services_s3_signing_corpus(
    #[files("fuzz/corpus/bedrock_services_s3_signing/*")] path: PathBuf,
) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the seed name is UTF-8");
    let bytes = fs::read(&path).expect("the seed reads");
    let expected = name.starts_with("valid") || name.starts_with("boundary-valid");
    let decoded = decode_upload_record(&bytes);
    assert_eq!(decoded.is_ok(), expected, "seed {name}");
}
