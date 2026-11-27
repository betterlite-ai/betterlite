use std::collections::BTreeMap;

use crate::aws_settings::sources::Credentials;
use crate::batch_call::status::{JobStatusCall, retrieve_model_job_status};
use crate::batch_call::transport::{ClientConfig, ControlPlaneRequest, header};

use super::fixture::{
    ACCESS_KEY, ClientConfigRecord, Factory, JOB_ID, REGION, SECRET_KEY, SESSION_TOKEN, Source,
    answer, credentials, credentials_with, now, runtime,
};

#[test]
fn bedrock_services_batch_call_credential_identity_isolation() {
    let first_credentials = credentials_with("AKIDFIRST", "first-secret", Some("first-token"));
    let second_credentials = credentials_with("AKIDSECOND", "second-secret", Some("second-token"));
    let first = signed_with(&first_credentials, "eu-west-1");
    let second = signed_with(&second_credentials, "ap-south-1");
    let again = signed_with(&first_credentials, "eu-west-1");
    assert_identity(
        (&first.0, &first.1),
        ("AKIDFIRST", "first-token", "eu-west-1"),
    );
    assert_identity(
        (&second.0, &second.1),
        ("AKIDSECOND", "second-token", "ap-south-1"),
    );
    assert_identity(
        (&again.0, &again.1),
        ("AKIDFIRST", "first-token", "eu-west-1"),
    );
}

fn signed_with(
    credentials: &Credentials,
    region: &str,
) -> (ControlPlaneRequest, ClientConfigRecord) {
    let source = Source::new(credentials.clone());
    let factory = Factory::new(vec![answer(200, r#"{"status":"InProgress"}"#)]);
    let kwargs = BTreeMap::new();
    let call = JobStatusCall {
        batch_id: JOB_ID,
        aws_region_name: Some(region),
        kwargs: &kwargs,
        now: now(),
    };
    runtime()
        .block_on(retrieve_model_job_status(call, &source, &factory, None))
        .expect("the lookup succeeds");
    let requests = factory.requests();
    let configs = factory.configs();
    (requests[0].clone(), configs[0].clone())
}

fn assert_identity(
    signed: (&ControlPlaneRequest, &ClientConfigRecord),
    expected: (&str, &str, &str),
) {
    let (request, config) = signed;
    let (access, token, region) = expected;
    assert_eq!(config.access_key_id, access);
    let authorization = header(&request.headers, "authorization").expect("the request is signed");
    assert!(authorization.contains(&format!("Credential={access}/")));
    assert!(authorization.contains(&format!("/{region}/bedrock/aws4_request")));
    assert_eq!(
        header(&request.headers, "x-amz-security-token"),
        Some(token)
    );
    assert!(request.url.contains(region));
}

#[test]
fn bedrock_services_batch_call_secret_safe_failure() {
    debug_hides_header_values();
    errors_hide_credentials();
}

fn debug_hides_header_values() {
    let request = ControlPlaneRequest {
        method: "GET".to_owned(),
        url: "https://bedrock.eu-west-1.amazonaws.com/model-invocation-job/job-1".to_owned(),
        headers: vec![
            (
                "authorization".to_owned(),
                format!(
                    "AWS4-HMAC-SHA256 Credential={ACCESS_KEY}/20231114/eu-west-1/bedrock/aws4_request"
                ),
            ),
            ("x-amz-security-token".to_owned(), SESSION_TOKEN.to_owned()),
        ],
        body: Vec::new(),
    };
    let shown = format!("{request:?}");
    assert!(!shown.contains(ACCESS_KEY));
    assert!(!shown.contains(SESSION_TOKEN));
    assert!(shown.contains("authorization"));
    let credentials = credentials();
    let config = ClientConfig {
        service_name: "bedrock",
        region: REGION,
        credentials: &credentials,
        signature_version: "v4",
    };
    let shown_config = format!("{config:?}");
    assert!(!shown_config.contains(ACCESS_KEY));
    assert!(!shown_config.contains(SECRET_KEY));
    assert!(!shown_config.contains(SESSION_TOKEN));
}

fn errors_hide_credentials() {
    let source = Source::failing();
    let factory = Factory::new(Vec::new());
    let kwargs = BTreeMap::new();
    let call = JobStatusCall {
        batch_id: JOB_ID,
        aws_region_name: Some(REGION),
        kwargs: &kwargs,
        now: now(),
    };
    let error = runtime()
        .block_on(retrieve_model_job_status(call, &source, &factory, None))
        .expect_err("the credential failure is reported");
    let shown = format!("{error:?} {error}");
    assert!(!shown.contains(ACCESS_KEY));
    assert!(!shown.contains(SECRET_KEY));
    assert!(!shown.contains(SESSION_TOKEN));
}
