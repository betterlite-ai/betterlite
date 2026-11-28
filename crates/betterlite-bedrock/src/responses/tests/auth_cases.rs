use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::ErrorHeaders;
use crate::credentials::error::CredentialError;
use crate::responses::auth::{SignInputs, SignedHeaders, sign_request, validate_environment};
use crate::responses::error::{ResponsesError, error_from_reply};
use crate::responses::url::{CompleteUrlParams, complete_url};
use crate::signing::target::RegionInputs;

use super::support::{block_on, secret};
use super::url_cases::blank;

const API_BASE: &str = "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1/responses";
const NOW_SECONDS: u64 = 1_767_225_600;
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

type Resolved = Ready<Result<Credentials, CredentialError>>;

fn credentials_of(access_key_id: &str) -> Credentials {
    Credentials {
        access_key_id: secret(access_key_id),
        secret_access_key: secret(SECRET_KEY),
        session_token: None,
    }
}

fn first_identity(_settings: &AwsSettings, _region: &str) -> Resolved {
    ready(Ok(credentials_of("AKIDFIRST")))
}

fn second_identity(_settings: &AwsSettings, _region: &str) -> Resolved {
    ready(Ok(credentials_of("AKIDSECOND")))
}

fn missing_credentials(_settings: &AwsSettings, _region: &str) -> Resolved {
    ready(Err(CredentialError::MissingCredentials))
}

fn never_resolved(_settings: &AwsSettings, _region: &str) -> Resolved {
    panic!("a bearer token never reaches the credential lookup")
}

fn headers_of(pairs: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Some((*value).to_owned())))
        .collect()
}

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .expect("the fixture clock is in range")
}

fn params_in(region: &str) -> BTreeMap<String, Box<RawValue>> {
    serde_json::from_str(&format!(r#"{{"aws_region_name": "{region}"}}"#)).expect("fixture params")
}

fn body_data() -> IndexMap<String, Json> {
    serde_json::from_str(r#"{"model": "m", "input": "hi"}"#).expect("fixture body")
}

fn header_value<'signed>(signed: &'signed SignedHeaders, name: &str) -> Option<&'signed str> {
    signed
        .headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_deref())
}

#[derive(Clone, Copy)]
struct Request<'request> {
    headers: &'request [(String, Option<String>)],
    params: &'request BTreeMap<String, Box<RawValue>>,
    data: &'request IndexMap<String, Json>,
    api_key: Option<&'request SecretString>,
    environment_token: Option<&'request SecretString>,
}

fn signed_with(
    request: &Request<'_>,
    resolve: fn(&AwsSettings, &str) -> Resolved,
) -> Result<SignedHeaders, ResponsesError> {
    block_on(sign_request(
        SignInputs {
            headers: request.headers,
            optional_params: request.params,
            request_data: request.data,
            api_base: API_BASE,
            model: None,
            api_key: request.api_key,
            environment_token: request.environment_token,
            region_inputs: RegionInputs::default(),
            now: now(),
        },
        resolve,
    ))
}

pub(super) fn responses_bearer_auth() {
    header_override();
    bearer_signing();
}

fn header_override() {
    let explicit = secret("explicit-token");
    let from_env = secret("env-token");
    let caller: IndexMap<String, String> = IndexMap::from([
        ("Authorization".to_owned(), "Bearer caller".to_owned()),
        ("X-Other".to_owned(), "kept".to_owned()),
    ]);
    let mut untouched = caller.clone();
    validate_environment(&mut untouched, Some(&secret("")), None);
    assert_eq!(untouched, caller);
    let empty = secret("");
    let mut untouched_pair = caller.clone();
    validate_environment(&mut untouched_pair, Some(&empty), Some(&empty));
    assert_eq!(untouched_pair, caller);
    let mut overridden = caller;
    validate_environment(&mut overridden, Some(&explicit), Some(&from_env));
    let pairs: Vec<(&str, &str)> = overridden
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("Authorization", "Bearer explicit-token"),
            ("X-Other", "kept")
        ]
    );
    let mut added: IndexMap<String, String> = IndexMap::new();
    validate_environment(&mut added, None, Some(&from_env));
    assert_eq!(
        added.get("Authorization").map(String::as_str),
        Some("Bearer env-token")
    );
}

fn bearer_signing() {
    let explicit = secret("explicit-token");
    let from_env = secret("env-token");
    let supplied = headers_of(&[("Content-Type", "application/json"), ("X-Other", "kept")]);
    let params = params_in("us-west-2");
    let data = body_data();
    let request = Request {
        headers: &supplied,
        params: &params,
        data: &data,
        api_key: Some(&explicit),
        environment_token: None,
    };
    let explicit_signed =
        signed_with(&request, never_resolved).expect("a bearer token skips signing");
    assert_eq!(explicit_signed.headers, supplied);
    assert_eq!(explicit_signed.body, None);
    let from_environment = Request {
        api_key: None,
        environment_token: Some(&from_env),
        ..request
    };
    let env_signed = signed_with(&from_environment, never_resolved)
        .expect("the environment token skips signing");
    assert_eq!(env_signed.headers, supplied);
    assert_eq!(env_signed.body, None);
    let empty = secret("");
    let all_empty = Request {
        api_key: Some(&empty),
        environment_token: Some(&empty),
        ..request
    };
    let signed = signed_with(&all_empty, first_identity).expect("an all-empty pair signs SigV4");
    assert!(header_value(&signed, "authorization").is_some());
    assert_eq!(header_value(&signed, "x-other"), Some("kept"));
    assert!(signed.body.is_some());
}

pub(super) fn responses_sigv4() {
    let supplied = headers_of(&[("content-type", "application/json"), ("x-custom", "keep")]);
    let params = params_in("us-west-2");
    let data = body_data();
    let empty = secret("");
    let request = Request {
        headers: &supplied,
        params: &params,
        data: &data,
        api_key: Some(&empty),
        environment_token: Some(&empty),
    };
    let signed = signed_with(&request, first_identity).expect("the request signs");
    let authorization = header_value(&signed, "authorization").expect("an Authorization header");
    assert!(authorization.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIDFIRST/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature="
    ));
    assert_eq!(
        header_value(&signed, "x-amz-date"),
        Some("20260101T000000Z")
    );
    assert_eq!(header_value(&signed, "x-custom"), Some("keep"));
    assert_eq!(
        signed.body.as_deref(),
        Some(br#"{"model": "m", "input": "hi"}"#.as_slice())
    );
    let failed = signed_with(&request, missing_credentials);
    assert!(matches!(failed, Err(ResponsesError::Signing(_))));
    let reply = [("x-amzn-RequestId".to_owned(), "req-1".to_owned())];
    let error = error_from_reply(429, "slow down", &reply).expect("ASCII headers");
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.message, "slow down");
    assert_eq!(error.headers, Some(ErrorHeaders::Http(reply.to_vec())));
    assert_eq!(error.response_headers, reply.to_vec());
}

pub(super) fn credential_identity_isolation() {
    let params_west = params_in("us-west-2");
    let params_east = params_in("eu-west-1");
    let supplied = headers_of(&[("content-type", "application/json")]);
    let data = body_data();
    let first = Request {
        headers: &supplied,
        params: &params_west,
        data: &data,
        api_key: None,
        environment_token: None,
    };
    let second = Request {
        params: &params_east,
        ..first
    };
    let first_signed = signed_with(&first, first_identity).expect("the first identity signs");
    let second_signed = signed_with(&second, second_identity).expect("the second identity signs");
    let first_auth = header_value(&first_signed, "authorization").expect("first Authorization");
    let second_auth = header_value(&second_signed, "authorization").expect("second Authorization");
    assert!(first_auth.contains("Credential=AKIDFIRST/20260101/us-west-2/"));
    assert!(second_auth.contains("Credential=AKIDSECOND/20260101/eu-west-1/"));
    assert!(!first_auth.contains("AKIDSECOND") && !second_auth.contains("AKIDFIRST"));
    let bearer_key = secret("first-identity-token");
    let bearer = Request {
        api_key: Some(&bearer_key),
        ..first
    };
    let bearer_signed = signed_with(&bearer, never_resolved).expect("a bearer token skips signing");
    assert!(header_value(&bearer_signed, "authorization").is_none());
    bearer_identities();
}

fn bearer_identities() {
    let first_key = secret("first-identity-token");
    let second_key = secret("second-identity-token");
    let mut first_headers: IndexMap<String, String> = IndexMap::new();
    let mut second_headers: IndexMap<String, String> = IndexMap::new();
    validate_environment(&mut first_headers, Some(&first_key), Some(&second_key));
    validate_environment(&mut second_headers, None, Some(&second_key));
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer first-identity-token")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer second-identity-token")
    );
}

pub(super) fn secret_safe_failure() {
    let token = "fixture-bearer-secret";
    let supplied = headers_of(&[("Authorization", "Bearer fixture-header-secret")]);
    let params: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"aws_region_name": "us-west-2", "aws_secret_access_key": "fixture-aws-secret"}"#,
    )
    .expect("fixture params");
    let data = body_data();
    let key = secret(token);
    let request = Request {
        headers: &supplied,
        params: &params,
        data: &data,
        api_key: Some(&key),
        environment_token: None,
    };
    let signed = signed_with(&request, never_resolved).expect("the bearer path signs nothing");
    assert_eq!(signed.headers, supplied);
    let region_error = complete_url(CompleteUrlParams {
        aws_region_name: Some("US-EAST-1"),
        ..blank()
    })
    .expect_err("an uppercase region is invalid");
    let no_token = Request {
        api_key: None,
        ..request
    };
    let Err(signing_error) = signed_with(&no_token, missing_credentials) else {
        panic!("the lookup fails");
    };
    let rendered =
        format!("{region_error} {region_error:?} {signing_error} {signing_error:?} {key:?}");
    for secret_text in [token, "fixture-header-secret", "fixture-aws-secret"] {
        assert!(!rendered.contains(secret_text));
    }
    assert!(matches!(
        region_error,
        ResponsesError::Region(AwsEndpointError::Region { .. })
    ));
}
