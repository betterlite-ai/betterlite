use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::support;
use crate::bedrock_settings::ErrorHeaders;
use crate::claude_platform_inputs::error::{PlatformInputError, error_class};
use crate::claude_platform_inputs::signing::{PlatformSignRequest, PlatformSignature};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

const API_BASE: &str = "https://aws-external-anthropic.eu-west-1.api.aws/v1/messages";
const NOW: u64 = 1_767_225_600;

fn raw_optional_params(entries: &[(&str, &str)]) -> BTreeMap<String, Box<RawValue>> {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                RawValue::from_string(format!("\"{value}\"")).unwrap(),
            )
        })
        .collect()
}

fn sign(request: PlatformSignRequest<'_>) -> Result<PlatformSignature, PlatformInputError> {
    support::sign_with(request, |_settings, _region| Ok(support::credentials()))
}

fn supplied_headers_of(signature: PlatformSignature) -> Vec<(String, Option<String>)> {
    let PlatformSignature::Supplied { headers } = signature else {
        panic!("expected supplied headers, signed request");
    };
    headers
}

fn signed(signature: PlatformSignature) -> AwsSignedRequest {
    match signature {
        PlatformSignature::Signed(signed) => signed,
        PlatformSignature::Supplied { headers } => {
            panic!("expected a signed request, got {} headers", headers.len())
        }
    }
}

fn header_map(headers: &[(String, String)]) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

pub(super) fn platform_signing() {
    explicit_key_supplies_headers();
    environment_key_supplies_headers();
    bearer_token_signs();
    sigv4_signs_for_the_platform_service();
    error_class_delegates();
}

fn supplied_headers() -> Vec<(String, Option<String>)> {
    vec![
        (
            "content-type".to_owned(),
            Some("application/json".to_owned()),
        ),
        ("x-custom".to_owned(), Some("keep".to_owned())),
        ("x-dropped".to_owned(), None),
    ]
}

fn sign_supplied(
    api_key: Option<&SecretString>,
    environment_api_key: Option<&SecretString>,
) -> Vec<(String, Option<String>)> {
    let headers = supplied_headers();
    let data = support::request_data();
    let optional_params = BTreeMap::new();
    let matched = sign(PlatformSignRequest {
        headers: &headers,
        optional_params: &optional_params,
        request_data: &data,
        api_base: API_BASE,
        api_key,
        environment_api_key,
        configured_bearer: None,
        model: None,
        region_inputs: RegionInputs::default(),
        now: support::epoch(NOW),
    })
    .unwrap();
    supplied_headers_of(matched)
}

fn explicit_key_supplies_headers() {
    let explicit = SecretString::from("explicit-fixture-key");
    let matched = sign_supplied(Some(&explicit), None);
    assert_eq!(matched, supplied_headers());
}

fn environment_key_supplies_headers() {
    let environment = SecretString::from("environment-fixture-key");
    let from_environment = sign_supplied(None, Some(&environment));
    assert_eq!(from_environment, supplied_headers());
    let empty = SecretString::from("");
    let empty_explicit = sign_supplied(Some(&empty), Some(&environment));
    assert_eq!(empty_explicit, supplied_headers());
}

fn bearer_token_signs() {
    let headers = vec![
        (
            "content-type".to_owned(),
            Some("application/json".to_owned()),
        ),
        ("x-custom".to_owned(), Some("keep".to_owned())),
    ];
    let data = support::request_data();
    let optional_params = BTreeMap::new();
    let empty = SecretString::from("");
    let bearer = SecretString::from("fixture-bearer-token");
    let signature = sign(PlatformSignRequest {
        headers: &headers,
        optional_params: &optional_params,
        request_data: &data,
        api_base: API_BASE,
        api_key: Some(&empty),
        environment_api_key: Some(&empty),
        configured_bearer: Some(&bearer),
        model: None,
        region_inputs: RegionInputs::default(),
        now: support::epoch(NOW),
    })
    .unwrap();
    let bearer_signed = signed(signature);
    assert_eq!(bearer_signed.method, "POST");
    assert_eq!(bearer_signed.url, API_BASE);
    assert_eq!(
        header_map(&bearer_signed.headers),
        BTreeMap::from([
            ("Authorization", "Bearer fixture-bearer-token"),
            ("Content-Type", "application/json"),
            ("content-type", "application/json"),
            ("x-custom", "keep"),
        ])
    );
}

fn sigv4_signs_for_the_platform_service() {
    let headers = vec![(
        "content-type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let data = support::request_data();
    let optional_params = raw_optional_params(&[("aws_region_name", "eu-west-1")]);
    let empty = SecretString::from("");
    let signature = sign(PlatformSignRequest {
        headers: &headers,
        optional_params: &optional_params,
        request_data: &data,
        api_base: API_BASE,
        api_key: Some(&empty),
        environment_api_key: None,
        configured_bearer: None,
        model: None,
        region_inputs: RegionInputs::default(),
        now: support::epoch(NOW),
    })
    .unwrap();
    let signed_request = signed(signature);
    let authorization = header_map(&signed_request.headers)
        .get("Authorization")
        .copied()
        .unwrap();
    assert!(authorization.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIDFIXTURE/20260101/eu-west-1/aws-external-anthropic/aws4_request"
    ));
    let body: Json = serde_json::from_slice(&signed_request.body).unwrap();
    let Json::Object(fields) = body else {
        panic!("the signed body is not an object")
    };
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text("claude-fixture".to_owned()))
    );
    assert_eq!(
        fields.get("max_tokens"),
        Some(&Json::Number(serde_json::Number::from(16_i64)))
    );
}

fn error_class_delegates() {
    let error = error_class(
        "fixture message".to_owned(),
        400,
        Some(ErrorHeaders::Http(vec![(
            "x-amzn-errortype".to_owned(),
            "fixture".to_owned(),
        )])),
    )
    .unwrap();
    assert_eq!(error.error.status_code, 400);
    assert_eq!(error.error.message, "fixture message");
    assert_eq!(
        error.response_headers,
        [("x-amzn-errortype".to_owned(), "fixture".to_owned())]
    );
    let no_headers = error_class("unauthorized".to_owned(), 401, None).unwrap();
    assert_eq!(no_headers.error.status_code, 401);
    assert!(no_headers.response_headers.is_empty());
}
