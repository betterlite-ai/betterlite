#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    AwsCredentialsResolver, AwsParams, FederationError, FederationInput,
    credentials_from_explicit_aws,
};

mod exchange;

const AUDIENCE: &str =
    "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/providers/x";
const SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:aws:token-type:aws4_request";
const TOKEN_URL: &str = "https://sts.googleapis.com/v1/token";
const FIXED_NOW_SECONDS: u64 = 1_767_225_600;
const GOLDEN_TOKEN: &str = "%7B%22headers%22%3A%5B%7B%22key%22%3A%22Authorization%22%2C%22value%22%3A%22AWS4-HMAC-SHA256%20Credential%3DAKIAEXAMPLE/20260101/us-east-1/sts/aws4_request%2C%20SignedHeaders%3Dhost%3Bx-amz-date%3Bx-amz-security-token%2C%20Signature%3Dc0407cd3a6ed9da8fc467107d5f00c27c89aaeaf3877ba0002f347ec3a3a0633%22%7D%2C%7B%22key%22%3A%22host%22%2C%22value%22%3A%22sts.us-east-1.amazonaws.com%22%7D%2C%7B%22key%22%3A%22x-amz-date%22%2C%22value%22%3A%2220260101T000000Z%22%7D%2C%7B%22key%22%3A%22x-amz-security-token%22%2C%22value%22%3A%22session-token-example%22%7D%2C%7B%22key%22%3A%22x-goog-cloud-target-resource%22%2C%22value%22%3A%22//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/providers/x%22%7D%5D%2C%22method%22%3A%22POST%22%2C%22url%22%3A%22https%3A//sts.us-east-1.amazonaws.com%3FAction%3DGetCallerIdentity%26Version%3D2011-06-15%22%7D";

struct RecordingResolver {
    issued: Mutex<VecDeque<Credentials>>,
    calls: AtomicUsize,
}

impl RecordingResolver {
    fn new(issued: Vec<Credentials>) -> Self {
        Self {
            issued: Mutex::new(VecDeque::from(issued)),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AwsCredentialsResolver for RecordingResolver {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let next = self
            .issued
            .lock()
            .expect("the issued queue is not poisoned")
            .pop_front()
            .expect("the test issued a credential for every resolution");
        async move { Ok(next) }
    }
}

fn credentials_with(access: &str, secret: &str, token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access),
        secret_access_key: SecretString::from(secret),
        session_token: token.map(SecretString::from),
    }
}

fn aws_params(region: Option<&str>) -> AwsParams {
    AwsParams {
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        region_name: region.map(str::to_owned),
        session_name: None,
        profile_name: None,
        role_name: None,
        web_identity_token: None,
        sts_endpoint: None,
        external_id: None,
    }
}

fn explicit_aws_params() -> AwsParams {
    let mut params = aws_params(Some("us-east-1"));
    params.access_key_id = Some(SecretString::from("AKIA-EXPLICIT"));
    params.secret_access_key = Some(SecretString::from("secret-explicit"));
    params
}

fn input(aws: AwsParams) -> FederationInput {
    FederationInput {
        audience: Some(AUDIENCE.to_owned()),
        subject_token_type: Some(SUBJECT_TOKEN_TYPE.to_owned()),
        token_url: Some(TOKEN_URL.to_owned()),
        impersonation_url: None,
        universe_domain: None,
        scopes: Vec::new(),
        aws,
    }
}

fn fixed_now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .expect("the fixed instant is representable")
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn region_before_aws(#[case] region: Option<&str>) {
    let resolver = RecordingResolver::new(Vec::new());
    let error = credentials_from_explicit_aws(input(aws_params(region)), &resolver)
        .err()
        .expect("a WIF credential without a region is refused");
    assert!(matches!(error, FederationError::AwsRegionRequired));
    assert_eq!(resolver.calls(), 0);
}

#[rstest]
#[tokio::test]
async fn rotating_session_token() {
    let resolver = RecordingResolver::new(vec![
        credentials_with("AKIA-FIRST", "secret-first", Some("session-first")),
        credentials_with("AKIA-SECOND", "secret-second", Some("session-second")),
    ]);
    let federation = credentials_from_explicit_aws(input(aws_params(Some("us-east-1"))), &resolver)
        .expect("the federation is built");
    let first = federation
        .subject_token(fixed_now())
        .await
        .expect("the first subject token is signed");
    let second = federation
        .subject_token(fixed_now())
        .await
        .expect("the second subject token is signed");
    assert!(first.expose_secret().contains("session-first"));
    assert!(second.expose_secret().contains("session-second"));
    assert!(!first.expose_secret().contains("session-second"));
    assert_ne!(first.expose_secret(), second.expose_secret());
    assert_eq!(resolver.calls(), 2);
}

#[rstest]
#[case::region_only(aws_params(Some("us-east-1")))]
#[case::explicit_keys(explicit_aws_params())]
#[tokio::test]
async fn no_metadata_source(#[case] params: AwsParams) {
    let resolver = RecordingResolver::new(vec![credentials_with(
        "AKIA-EXPLICIT",
        "secret-explicit",
        None,
    )]);
    let federation = credentials_from_explicit_aws(input(params), &resolver)
        .expect("explicit AWS federation is built");
    let token = federation
        .subject_token(fixed_now())
        .await
        .expect("the subject token is signed");
    assert_eq!(resolver.calls(), 1);
    assert!(!token.expose_secret().contains("169.254.169.254"));
    assert!(
        token
            .expose_secret()
            .contains("sts.us-east-1.amazonaws.com")
    );
    assert!(
        token
            .expose_secret()
            .contains("x-goog-cloud-target-resource")
    );
}

#[rstest]
#[case::supplied(
    Some(
        "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/sa@example.iam.gserviceaccount.com:generateAccessToken"
    ),
    Some("example.com")
)]
#[case::omitted(None, None)]
#[tokio::test]
async fn impersonation_and_universe(
    #[case] impersonation: Option<&str>,
    #[case] universe: Option<&str>,
) {
    let resolver = RecordingResolver::new(vec![credentials_with("AKIA", "secret", None)]);
    let mut federation_input = input(aws_params(Some("us-east-1")));
    federation_input.impersonation_url = impersonation.map(str::to_owned);
    federation_input.universe_domain = universe.map(str::to_owned);
    let federation = credentials_from_explicit_aws(federation_input, &resolver)
        .expect("the federation is built");
    assert_eq!(federation.impersonation_url.as_deref(), impersonation);
    assert_eq!(federation.universe_domain.as_deref(), universe);
    assert_eq!(federation.audience.as_deref(), Some(AUDIENCE));
    assert_eq!(
        federation.subject_token_type.as_deref(),
        Some(SUBJECT_TOKEN_TYPE)
    );
    assert_eq!(federation.token_url.as_deref(), Some(TOKEN_URL));
    assert_eq!(
        federation.supplier.settings.region_name.as_deref(),
        Some("us-east-1")
    );
}

#[rstest]
#[case::scoped(vec!["https://www.googleapis.com/auth/cloud-platform".to_owned()])]
#[case::unscoped(Vec::new())]
#[tokio::test]
async fn scoped_supplier(#[case] scopes: Vec<String>) {
    let resolver = RecordingResolver::new(vec![credentials_with("AKIA", "secret", None)]);
    let mut federation_input = input(aws_params(Some("us-east-1")));
    federation_input.scopes = scopes.clone();
    let federation = credentials_from_explicit_aws(federation_input, &resolver)
        .expect("the federation is built");
    assert_eq!(federation.scopes, scopes);
    let token = federation
        .subject_token(fixed_now())
        .await
        .expect("the subject token is signed");
    assert!(token.expose_secret().contains("AKIA"));
}

#[rstest]
#[tokio::test]
async fn subject_token_matches_google_auth() {
    let resolver = RecordingResolver::new(vec![credentials_with(
        "AKIAEXAMPLE",
        "secret/example",
        Some("session-token-example"),
    )]);
    let federation = credentials_from_explicit_aws(input(aws_params(Some("us-east-1"))), &resolver)
        .expect("the federation is built");
    let token = federation
        .subject_token(fixed_now())
        .await
        .expect("the subject token is signed");
    assert_eq!(token.expose_secret(), GOLDEN_TOKEN);
}
