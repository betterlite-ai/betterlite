use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;

use super::fixture::{Source, credentials, environment, headers, no_secrets, now, runtime};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::credentials::{CredentialRequest, load_credentials};
use crate::bedrock_embedding_io::signed_get::sign_get_request;

const URL: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/amazon.titan-embed-text-v1/invoke";

fn resolved(access: &str, secret_key: &str, token: &str, region: &str) -> (Credentials, String) {
    let source = Source::new(credentials(access, secret_key, Some(token)));
    let mut parameters = BTreeMap::new();
    let loaded = runtime()
        .block_on(load_credentials(
            CredentialRequest {
                optional_params: &mut parameters,
                environment: &environment(&[("AWS_REGION_NAME", region)]),
                bearer_token: None,
            },
            &secret_lookup(None),
            source.resolve(),
        ))
        .expect("the fixture credentials load");
    assert_eq!(source.calls().len(), 1);
    let resolved = loaded
        .credentials
        .expect("the resolver supplies credentials");
    assert_eq!(resolved.access_key_id.expose_secret().as_str(), access);
    assert_eq!(
        resolved
            .session_token
            .as_ref()
            .map(|value| value.expose_secret().as_str()),
        Some(token)
    );
    (resolved, loaded.region)
}

fn signed(identity: &Credentials, region: &str) -> BTreeMap<String, String> {
    sign_get_request(
        identity,
        URL,
        &headers(&[("Content-Type", "application/json")]),
        region,
        now(),
    )
    .expect("the fixture request signs")
    .headers
    .into_iter()
    .collect()
}

#[test]
fn bedrock_embedding_io_credential_identity_isolation() {
    let (first, first_region) = resolved(
        "AKIAFIRSTIDENTITY",
        "first-secret-key",
        "first-token",
        "eu-west-1",
    );
    let (second, second_region) = resolved(
        "AKIASECONDIDENTITY",
        "second-secret-key",
        "second-token",
        "us-east-2",
    );
    let first_headers = signed(&first, &first_region);
    let second_headers = signed(&second, &second_region);
    assert_ne!(
        first_headers["Authorization"],
        second_headers["Authorization"]
    );
    assert!(
        first_headers["Authorization"].contains("AKIAFIRSTIDENTITY"),
        "the first identity signs with its own access key"
    );
    assert!(
        second_headers["Authorization"].contains("AKIASECONDIDENTITY"),
        "the second identity signs with its own access key"
    );
    assert!(!first_headers["Authorization"].contains("AKIASECONDIDENTITY"));
    assert!(!first_headers["Authorization"].contains("us-east-2"));
    assert_eq!(first_headers["X-Amz-Security-Token"], "first-token");
    assert_eq!(second_headers["X-Amz-Security-Token"], "second-token");
    let (again, again_region) = resolved(
        "AKIAFIRSTIDENTITY",
        "first-secret-key",
        "first-token",
        "eu-west-1",
    );
    assert_eq!(again_region, first_region);
    assert_eq!(
        signed(&again, &again_region),
        first_headers,
        "a later identity resolves no cached material of its own"
    );
    assert_ne!(second_region, first_region);
}
