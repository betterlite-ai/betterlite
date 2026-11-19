use std::error::Error;
use std::future::ready;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json as ContractJson;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::invoke_primitives::config::Params as InvokeParams;
use crate::messages::tests::support::{Keys, Signing, contract, header, sign};
use crate::messages::transport::{MessagesSignRequest, sign_messages_request};
use crate::signing::target::RegionInputs;

const MODEL: &str = "fixture-plain";
const API_BASE: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/fixture/invoke";

fn signing<'request>(
    headers: &'request IndexMap<String, String>,
    data: &'request IndexMap<String, ContractJson>,
    api_key: Option<&'request SecretString>,
) -> Signing<'request> {
    Signing {
        model: MODEL,
        headers,
        request_data: data,
        api_base: API_BASE,
        api_key,
        configured_bearer: None,
    }
}

fn keys(
    access_key_id: &'static str,
    secret_access_key: &'static str,
    session_token: Option<&'static str>,
) -> Keys {
    Keys {
        access_key_id,
        secret_access_key,
        session_token,
    }
}

#[test]
fn bedrock_messages_credential_identity_isolation() {
    let headers = IndexMap::new();
    let data = contract("{}");
    let first = sign(
        signing(&headers, &data, None),
        &keys("AKIAFIRST", "first-secret", None),
    )
    .expect("the first identity signs");
    let second = sign(
        signing(&headers, &data, None),
        &keys("AKIASECOND", "second-secret", Some("second-session")),
    )
    .expect("the second identity signs");
    let first_auth = header(&first.headers, "Authorization").expect("the first signature");
    let second_auth = header(&second.headers, "Authorization").expect("the second signature");
    assert!(first_auth.contains("Credential=AKIAFIRST/"));
    assert!(second_auth.contains("Credential=AKIASECOND/"));
    assert!(!second_auth.contains("AKIAFIRST"));
    assert_ne!(first_auth, second_auth);
    identity_via_api_key(&headers, &data);
}

fn identity_via_api_key(headers: &IndexMap<String, String>, data: &IndexMap<String, ContractJson>) {
    let bearer = SecretString::from("fixture-api-key");
    let signed = sign(
        signing(headers, data, Some(&bearer)),
        &keys("AKIAFIRST", "first-secret", None),
    )
    .expect("the configured identity signs");
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some("Bearer fixture-api-key")
    );
    assert_eq!(header(&signed.headers, "X-Amz-Date"), None);
}

#[test]
fn bedrock_messages_secret_safe_failure() {
    signed_debug_is_redacted();
    resolution_failure_is_clean();
}

fn signed_debug_is_redacted() {
    let headers = IndexMap::new();
    let data = contract("{}");
    let signed = sign(
        signing(&headers, &data, None),
        &keys(
            "AKIAFIXTURE",
            "fixture-secret",
            Some("fixture-session-token"),
        ),
    )
    .expect("the fixture signs");
    let debug = format!("{signed:?}");
    assert!(!debug.contains("fixture-secret"));
    assert!(!debug.contains("fixture-session-token"));
    assert!(debug.contains("Authorization"));
}

fn resolution_failure_is_clean() {
    let headers = IndexMap::new();
    let data = contract("{}");
    let params = InvokeParams::new();
    let request = MessagesSignRequest {
        headers: &headers,
        optional_params: &params,
        request_data: &data,
        api_base: API_BASE,
        api_key: None,
        configured_bearer: None,
        model: Some(MODEL),
        stream: None,
        fake_stream: None,
        region_inputs: RegionInputs::default(),
        now: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    };
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the test runtime builds");
    let error = runtime
        .block_on(sign_messages_request(request, |_settings, _region| {
            ready(Err::<Credentials, CredentialError>(
                CredentialError::MissingCredentials,
            ))
        }))
        .expect_err("credential resolution fails");
    let message = error.to_string();
    assert_eq!(message, "the AWS credential lookup failed");
    let cause = Error::source(&error).expect("the lookup error stays the source");
    assert_eq!(
        cause.to_string(),
        "the STS assume role response has no credentials"
    );
    assert!(!cause.to_string().contains("fixture-secret"));
}
