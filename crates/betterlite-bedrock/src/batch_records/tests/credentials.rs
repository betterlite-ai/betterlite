use betterlite_openai::files::records::CreatePurpose;
use secrecy::SecretString;

use super::support::{
    ACCESS_KEY, Families, GATEWAY_MODEL, OTHER_ACCESS_KEY, OTHER_SECRET_KEY, SECRET_KEY,
    batch_file, booking_params, param_text, run_upload,
};
use crate::batch_records::error::BatchRecordsError;
use crate::batch_records::upload::FileUploadHandoff;
use crate::request_policy::{Params as RequestParams, RequestValue};

#[test]
fn batch_records_credential_identity_isolation() {
    let file = batch_file();
    let (first, first_params) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    let first_handoff = first.expect("the first identity uploads");
    let mut other = booking_params();
    other.insert(
        String::from("s3_access_key_id"),
        RequestValue::Text(String::from(OTHER_ACCESS_KEY)),
    );
    other.insert(
        String::from("s3_secret_access_key"),
        RequestValue::Secret(SecretString::from(OTHER_SECRET_KEY)),
    );
    let (second, second_params) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut other,
    );
    let second_handoff = second.expect("the second identity uploads");
    let authorization = |handoff: &FileUploadHandoff| {
        handoff
            .headers
            .iter()
            .find(|(name, _value)| name == "Authorization")
            .expect("the upload is signed")
            .1
            .clone()
    };
    let first_authorization = authorization(&first_handoff);
    let second_authorization = authorization(&second_handoff);
    assert_ne!(first_authorization, second_authorization);
    assert!(first_authorization.contains(ACCESS_KEY));
    assert!(!first_authorization.contains(OTHER_ACCESS_KEY));
    assert!(second_authorization.contains(OTHER_ACCESS_KEY));
    assert!(!second_authorization.contains(ACCESS_KEY));
    // the same object with the same id is the same URL: nothing about identity leaks into it
    assert_eq!(first_handoff.url, second_handoff.url);
    assert_eq!(
        param_text(&first_params, "s3_access_key_id"),
        Some(ACCESS_KEY)
    );
    assert_eq!(
        param_text(&second_params, "s3_access_key_id"),
        Some(OTHER_ACCESS_KEY)
    );
    assert!(!format!("{:?}", first_params).contains(SECRET_KEY));
    assert!(!format!("{:?}", second_params).contains(OTHER_SECRET_KEY));
}

#[test]
fn batch_records_secret_safe_failure() {
    let file = batch_file();
    let mut without_bucket = RequestParams::new();
    without_bucket.insert(
        String::from("s3_access_key_id"),
        RequestValue::Text(String::from(ACCESS_KEY)),
    );
    without_bucket.insert(
        String::from("s3_secret_access_key"),
        RequestValue::Secret(SecretString::from(SECRET_KEY)),
    );
    without_bucket.insert(
        String::from("model"),
        RequestValue::Text(String::from(GATEWAY_MODEL)),
    );
    let (error, params) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut without_bucket,
    );
    let failure = error.expect_err("the upload without a bucket is refused");
    assert!(matches!(failure, BatchRecordsError::Signing(_)));
    assert!(!failure.to_string().contains(SECRET_KEY));
    assert!(!format!("{failure:?}").contains(SECRET_KEY));
    assert!(!format!("{params:?}").contains(SECRET_KEY));
    let (handoff, _after) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    let signed = handoff.expect("the upload signs");
    // Debug names the headers and the data length, never a signature or a key
    let rendered = format!("{signed:?}");
    assert!(!rendered.contains(SECRET_KEY));
    assert!(!rendered.contains("Authorization"));
    assert!(rendered.contains("header_names"));
    assert!(rendered.contains("data_length"));
}
