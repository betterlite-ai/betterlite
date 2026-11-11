use std::collections::BTreeMap;
use std::future::ready;

use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::invoke_primitives::config::Params;
use crate::invoke_primitives::signing::{SignRequest, sign_request};
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

use super::support::{credentials, epoch, params, request_data, runtime};

const ENDPOINT: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude/invoke";
const NOW: i64 = 1_767_225_600;
const MODEL: &str = "anthropic.claude-v2";

#[derive(Clone, Copy, Default)]
struct Flags {
    stream: Option<bool>,
    fake_stream: Option<bool>,
}

fn request<'text>(
    headers: &'text [(String, Option<String>)],
    optional_params: &'text Params,
    data: &'text IndexMap<String, Json>,
    api_key: Option<&'text SecretString>,
    flags: Flags,
) -> SignRequest<'text> {
    SignRequest {
        headers,
        optional_params,
        request_data: data,
        api_base: ENDPOINT,
        api_key,
        configured_bearer: None,
        model: Some(MODEL),
        stream: flags.stream,
        fake_stream: flags.fake_stream,
        region_inputs: RegionInputs::default(),
        now: epoch(NOW),
    }
}

fn sign(request: SignRequest<'_>, fixture: Credentials) -> Result<AwsSignedRequest, SigningError> {
    runtime().block_on(sign_request(request, move |_settings, _region| {
        ready(Ok::<Credentials, CredentialError>(fixture))
    }))
}

fn authorization(signed: &AwsSignedRequest) -> &str {
    signed
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str())
        .unwrap()
}

fn body(signed: &AwsSignedRequest) -> &[u8] {
    signed.body.as_slice()
}

pub(super) fn signing_forwarding() {
    delegated_signature_matches();
    stream_flags_change_nothing();
    api_key_is_the_bearer_token();
}

fn fixture() -> Credentials {
    credentials("AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY")
}

fn optional_params() -> Params {
    params(r#"{"temperature":0.2,"aws_region_name":"us-west-2"}"#)
}

fn data() -> IndexMap<String, Json> {
    request_data(r#"{"inputText":"hello","textGenerationConfig":{"temperature":0.2}}"#)
}

fn header_pairs() -> Vec<(String, Option<String>)> {
    vec![
        (
            "content-type".to_owned(),
            Some("application/json".to_owned()),
        ),
        ("x-custom".to_owned(), Some("keep".to_owned())),
    ]
}

fn delegated_signature_matches() {
    let headers = header_pairs();
    let signed = sign(
        request(
            &headers,
            &optional_params(),
            &data(),
            None,
            Flags::default(),
        ),
        fixture(),
    )
    .unwrap();
    let expected = service_signature(&headers);
    assert_eq!(signed.method, expected.method);
    assert_eq!(signed.url, expected.url);
    assert_eq!(signed.headers, expected.headers);
    assert_eq!(body(&signed), body(&expected));
    let signed_authorization = authorization(&signed);
    assert!(signed_authorization.starts_with("AWS4-HMAC-SHA256 "));
    assert!(
        signed_authorization
            .contains("Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request")
    );
    assert_eq!(body(&signed), dumps_object(&data()).as_bytes());
    assert!(
        signed
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-custom") && value == "keep")
    );
}

fn service_signature(headers: &[(String, Option<String>)]) -> AwsSignedRequest {
    let raw_optional: BTreeMap<String, Box<RawValue>> = optional_params()
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    runtime()
        .block_on(sign_service_request(
            ServiceRequest {
                service: ServiceName::Bedrock,
                headers,
                optional_params: &raw_optional,
                request_data: &data(),
                api_base: ENDPOINT,
                model: Some(MODEL),
                api_key: None,
                configured_bearer: None,
                region_inputs: RegionInputs::default(),
                now: epoch(NOW),
            },
            |_settings, _region| ready(Ok::<Credentials, CredentialError>(fixture())),
        ))
        .unwrap()
}

fn stream_flags_change_nothing() {
    let headers = header_pairs();
    let baseline = sign(
        request(
            &headers,
            &optional_params(),
            &data(),
            None,
            Flags::default(),
        ),
        fixture(),
    )
    .unwrap();
    let flag_sets = [
        Flags {
            stream: Some(true),
            fake_stream: Some(true),
        },
        Flags {
            stream: Some(false),
            fake_stream: None,
        },
        Flags {
            stream: None,
            fake_stream: Some(false),
        },
    ];
    for flags in flag_sets {
        let other = sign(
            request(&headers, &optional_params(), &data(), None, flags),
            fixture(),
        )
        .unwrap();
        assert_eq!(other.headers, baseline.headers);
        assert_eq!(other.body, baseline.body);
    }
}

fn api_key_is_the_bearer_token() {
    let headers = header_pairs();
    let bearer = SecretString::from("fixture-bearer".to_owned());
    let signed = runtime()
        .block_on(sign_request(
            request(
                &headers,
                &optional_params(),
                &data(),
                Some(&bearer),
                Flags::default(),
            ),
            |_settings, _region| {
                ready(Err::<Credentials, CredentialError>(
                    CredentialError::MissingCredentials,
                ))
            },
        ))
        .unwrap();
    assert_eq!(authorization(&signed), "Bearer fixture-bearer");
    assert_eq!(body(&signed), dumps_object(&data()).as_bytes());
}
