use std::collections::BTreeMap;
use std::future::{Future, Ready, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::files::records::FileObject;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;

use super::super::prefix::setting_text;
use super::super::requests::S3FilesRequest;
use super::super::responses::{
    S3RawResponse, S3ResponseContext, S3ResponseRequest, list_files_response,
};
use super::super::{LIST_FILES_LOCATION_PARAM, LIST_FILES_PURPOSE_PARAM, S3ListingError};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::{S3AccessParams, TrustedValue, frozen_mapping};
use crate::s3_signing::SIGNED_HEADERS_PARAM;
use crate::signing::target::RegionInputs;

pub(super) const SIGNING_SECONDS: u64 = 1_704_067_200;
pub(super) const EMPTY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub(super) fn fixed_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SIGNING_SECONDS))
        .unwrap()
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

pub(super) fn params(entries: Vec<(&str, RequestValue)>) -> Params {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

pub(super) fn null_purpose() -> Params {
    params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Raw("null".to_owned()),
    )])
}

pub(super) fn text_params(entries: &[(&str, &str)]) -> Params {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), RequestValue::Text((*value).to_owned())))
        .collect()
}

pub(super) fn region_params() -> Params {
    text_params(&[("aws_region_name", "us-east-1")])
}

pub(super) fn empty_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(super) fn env_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn region_inputs(environment: &BTreeMap<String, String>) -> RegionInputs<'_> {
    RegionInputs {
        environment_region_name: environment.get("AWS_REGION_NAME").map(String::as_str),
        environment_region: environment.get("AWS_REGION").map(String::as_str),
        session_region: None,
    }
}

pub(super) fn access_params(trusted: &[(&str, TrustedValue)]) -> S3AccessParams {
    S3AccessParams {
        trusted: frozen_mapping(
            trusted
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone())),
        ),
        bucket_name: None,
        output_bucket_name: None,
    }
}

pub(super) fn input_output_access() -> S3AccessParams {
    access_params(&[
        ("s3_bucket_name", TrustedValue::Text("in-bucket".to_owned())),
        (
            "s3_output_bucket_name",
            TrustedValue::Text("out-bucket".to_owned()),
        ),
    ])
}

pub(super) fn bucket_pair_access() -> S3AccessParams {
    access_params(&[
        ("s3_bucket_name", TrustedValue::Text("bucket/in".to_owned())),
        (
            "s3_output_bucket_name",
            TrustedValue::Text("bucket/out".to_owned()),
        ),
    ])
}

pub(super) fn null_location_params() -> Params {
    params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ])
}

pub(super) fn batch_params() -> Params {
    params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ])
}

pub(super) fn bucket_access(bucket: &str) -> S3AccessParams {
    access_params(&[("s3_bucket_name", TrustedValue::Text(bucket.to_owned()))])
}

pub(super) fn list_access() -> S3AccessParams {
    bucket_access("bucket")
}

pub(super) fn credentials(token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIDEXAMPLE".to_owned()),
        secret_access_key: SecretString::from("secret".to_owned()),
        session_token: token.map(|value| SecretString::from(value.to_owned())),
    }
}

pub(super) fn environment_credentials(
    _settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    ready(Ok(credentials(None)))
}

pub(super) fn token_credentials(
    _settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    ready(Ok(credentials(Some("tokEXAMPLE"))))
}

pub(super) fn unavailable_credentials(
    _settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    ready(Err(CredentialError::MissingCredentials))
}

pub(super) fn header_pairs(params: &Params) -> Vec<(String, String)> {
    match params.get(SIGNED_HEADERS_PARAM) {
        Some(RequestValue::Mapping(headers)) => headers
            .iter()
            .map(|(name, value)| match value {
                RequestValue::Text(text) => (name.clone(), text.clone()),
                RequestValue::Raw(raw) => (name.clone(), raw.clone()),
                RequestValue::Secret(secret) => (name.clone(), secret.expose_secret().to_owned()),
                RequestValue::Mapping(_) => (name.clone(), String::new()),
            })
            .collect(),
        Some(RequestValue::Text(_) | RequestValue::Raw(_) | RequestValue::Secret(_)) | None => {
            Vec::new()
        }
    }
}

pub(super) fn header_value(params: &Params, name: &str) -> Option<String> {
    header_pairs(params)
        .into_iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value)
}

pub(super) fn authorization(
    access_key: &str,
    region: &str,
    signed_headers: &str,
    signature: &str,
) -> String {
    format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/20240101/{region}/s3/aws4_request, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

pub(super) fn listing_request<'input>(
    optional_params: &'input Params,
    litellm_params: &'input mut Params,
    access: &'input S3AccessParams,
    environment: &'input BTreeMap<String, String>,
) -> S3FilesRequest<'input> {
    S3FilesRequest {
        optional_params,
        litellm_params,
        access,
        environment,
        region_inputs: region_inputs(environment),
        now: fixed_now(),
    }
}

pub(super) fn stored_text(params: &Params, name: &str) -> Option<String> {
    setting_text(params, name)
}

pub(super) fn listing_xml(contents: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>in-bucket</Name>{contents}</ListBucketResult>"
    )
    .into_bytes()
}

pub(super) fn contents_entry(key: &str, size: &str, last_modified: &str) -> String {
    format!(
        "<Contents><Key>{key}</Key><Size>{size}</Size><LastModified>{last_modified}</LastModified></Contents>"
    )
}

pub(super) fn raw_listing<'input>(
    status: u16,
    body: &'input [u8],
    headers: &'input [(String, String)],
) -> S3RawResponse<'input> {
    S3RawResponse {
        status,
        body,
        headers,
        request: S3ResponseRequest {
            method: "GET",
            url: "https://s3.us-east-1.amazonaws.com/in-bucket/?list-type=2",
        },
    }
}

pub(super) fn managed_files(
    body: &[u8],
    litellm_params: &Params,
) -> Result<Vec<FileObject>, S3ListingError> {
    let access = bucket_access("in-bucket");
    let environment = empty_environment();
    let context = S3ResponseContext {
        litellm_params,
        access: &access,
        environment: &environment,
    };
    list_files_response(&raw_listing(200, body, &[]), &context)
}
