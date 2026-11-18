use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::REDACTED;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::value::RawValue;

use crate::bedrock_embedding_io::transport::TransferError;

use super::super::error::EmbeddingJobsError;
use super::super::transport::StatusWireRequest;
use super::fixture::{
    ACCESS_KEY, ENCODED_ARN, INVOCATION_ARN, Reply, SECRET_KEY, SESSION_TOKEN, Scenario,
    credentials_with,
};

#[derive(Clone, Copy)]
enum Case {
    Url,
    Passthrough,
    HttpFailure,
    Logging,
}

#[rstest]
#[case::async_invoke_status_url(Case::Url)]
#[case::status_body_passthrough(Case::Passthrough)]
#[case::status_http_failure(Case::HttpFailure)]
#[case::status_logging(Case::Logging)]
fn embedding_jobs_cases(#[case] case: Case) {
    match case {
        Case::Url => url_case(),
        Case::Passthrough => passthrough_case(),
        Case::HttpFailure => http_failure_case(),
        Case::Logging => logging_case(),
    }
}

fn header<'request>(request: &'request StatusWireRequest, name: &str) -> &'request str {
    request
        .headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .map(|(_, value)| value.as_str())
        .expect("the fixture header was sent")
}

fn header_names(request: &StatusWireRequest) -> Vec<&str> {
    request
        .headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect()
}

fn object(value: &Json) -> &IndexMap<String, Json> {
    let Json::Object(fields) = value else {
        panic!("the fixture value is an object");
    };
    fields
}

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    object(value).get(name).expect("the fixture field exists")
}

fn text(value: &Json) -> &str {
    let Json::Text(text) = value else {
        panic!("the fixture value is text");
    };
    text
}

fn additional_args(snapshot: &Json) -> &Json {
    field(snapshot, "additional_args")
}

fn complete_input() -> Json {
    serde_json::from_str(&format!("{{\"invocation_arn\":\"{INVOCATION_ARN}\"}}"))
        .expect("the fixture object is JSON")
}

fn url_case() {
    let mut scenario = Scenario::new(Reply::ok("{}"));
    scenario.endpoint = Some("https://runtime.example///".to_owned());
    let run = super::fixture::run(&scenario);
    run.result.as_ref().expect("the status lookup succeeds");
    let request = run.requests.first().expect("the GET request was sent");
    assert_eq!(
        request.url,
        format!("https://runtime.example/async-invoke/{ENCODED_ARN}")
    );
    assert_signed_headers(request);
    // credential loading follows the environment region; the signature follows the method argument
    assert_eq!(
        run.resolved.first().map(|(_, region)| region.as_str()),
        Some("eu-west-1")
    );
    assert_eq!(run.factory_calls, vec![("bedrock".to_owned(), None)]);
    endpoint_precedence();
}

fn assert_signed_headers(request: &StatusWireRequest) {
    assert_eq!(
        header_names(request),
        [
            "Content-Type",
            "X-Amz-Date",
            "X-Amz-Security-Token",
            "Authorization"
        ]
    );
    assert_eq!(header(request, "Content-Type"), "application/json");
    assert_eq!(header(request, "X-Amz-Date"), "20231114T221320Z");
    assert!(
        header(request, "Authorization")
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-west-2/bedrock/")
    );
    assert!(
        header(request, "Authorization")
            .contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token")
    );
}

fn endpoint_precedence() {
    let mut environment_scenario = Scenario::new(Reply::ok("{}"));
    environment_scenario.environment.insert(
        "AWS_BEDROCK_RUNTIME_ENDPOINT".to_owned(),
        "https://env.example//".to_owned(),
    );
    let environment_run = super::fixture::run(&environment_scenario);
    assert_eq!(
        environment_run.requests[0].url,
        format!("https://env.example/async-invoke/{ENCODED_ARN}")
    );
    let mut default_scenario = Scenario::new(Reply::ok("{}"));
    default_scenario.environment.clear();
    let default_run = super::fixture::run(&default_scenario);
    assert_eq!(
        default_run.requests[0].url,
        format!("https://bedrock-runtime.us-west-2.amazonaws.com/async-invoke/{ENCODED_ARN}")
    );
}

fn passthrough_case() {
    passthrough_bodies();
    malformed_bodies_fail();
}

fn passthrough_bodies() {
    let failed = r#"{"status":"Failed","metadata":null,"unknown":{"s3":"s3://bucket/output"},"nested":[1,true,null]}"#;
    let run = super::fixture::run(&Scenario::new(Reply::ok(failed)));
    let value = run
        .result
        .expect("a Failed job in a 200 body is a lookup success");
    assert_eq!(
        value,
        serde_json::from_str::<Json>(failed).expect("the fixture body is JSON")
    );
    assert_eq!(run.requests.len(), 1);
    assert_eq!(field(&value, "status"), &Json::Text("Failed".to_owned()));
    assert_eq!(
        field(field(&value, "unknown"), "s3"),
        &Json::Text("s3://bucket/output".to_owned())
    );
    passthrough_body(
        r#"{"status":"Completed","output":{"s3Uri":"s3://bucket/out"}}"#,
        "a Completed job returns its body",
    );
    passthrough_body(
        r#"{"status":"InProgress","unknown":{"s3":"s3://bucket/x"}}"#,
        "an InProgress job returns its body",
    );
    for nonobject in ["[1,2]", "42", "\"text\"", "null"] {
        assert_eq!(
            super::fixture::run(&Scenario::new(Reply::ok(nonobject)))
                .result
                .expect("nonobject JSON passes through"),
            serde_json::from_str::<Json>(nonobject).expect("the fixture body is JSON"),
            "body {nonobject}"
        );
    }
}

fn passthrough_body(body: &str, message: &str) {
    assert_eq!(
        super::fixture::run(&Scenario::new(Reply::ok(body)))
            .result
            .expect(message),
        serde_json::from_str::<Json>(body).expect("the fixture body is JSON")
    );
}

fn malformed_bodies_fail() {
    for malformed in ["", "{\"status\":", "not json"] {
        assert!(
            matches!(
                super::fixture::run(&Scenario::new(Reply::ok(malformed))).result,
                Err(EmbeddingJobsError::Decode { .. })
            ),
            "body {malformed:?}"
        );
    }
}

fn http_failure_case() {
    status_messages();
    logged_and_transport_failures();
}

fn status_messages() {
    let run = super::fixture::run(&Scenario::new(Reply::status(500, "boom")));
    let error = run.result.expect_err("a 500 is a status failure");
    assert!(matches!(
        &error,
        EmbeddingJobsError::HttpFailure { status: 500, .. }
    ));
    assert_eq!(
        error.to_string(),
        "Failed to get async invoke status: 500 - boom"
    );
    for status in [201_u16, 204, 302] {
        let status_error = super::fixture::run(&Scenario::new(Reply::status(status, "created")))
            .result
            .expect_err("every non-200 status fails");
        assert_eq!(
            status_error.to_string(),
            format!("Failed to get async invoke status: {status} - created")
        );
    }
    // httpx's response.text replaces each invalid sequence with one U+FFFD
    let invalid_body = super::fixture::run(&Scenario::new(Reply::bytes(500, b"bad \xff \xe2\x82")));
    assert_eq!(
        invalid_body
            .result
            .expect_err("a 500 is a status failure")
            .to_string(),
        "Failed to get async invoke status: 500 - bad \u{fffd} \u{fffd}"
    );
}

fn logged_and_transport_failures() {
    let mut logged_scenario = Scenario::new(Reply::status(503, "unavailable"));
    logged_scenario.logging = true;
    let logged = super::fixture::run(&logged_scenario);
    assert!(logged.result.is_err(), "the 503 fails");
    assert_eq!(logged.snapshots.len(), 2, "both callbacks ran");
    assert_eq!(
        field(&logged.snapshots[1], "original_response"),
        &Json::Text("unavailable".to_owned())
    );
    assert!(matches!(
        super::fixture::run(&Scenario::new(Reply::Failed)).result,
        Err(EmbeddingJobsError::Transport(TransferError::Failed(
            TransportError::Closed
        )))
    ));
    let mut unsigned = Scenario::new(Reply::ok("{}"));
    unsigned.endpoint = Some("https://invalid host.example".to_owned());
    assert!(matches!(
        super::fixture::run(&unsigned).result,
        Err(EmbeddingJobsError::Signing(_))
    ));
}

fn logging_scenario() -> Scenario {
    let mut scenario = Scenario::new(Reply::ok(r#"{"status":"Completed"}"#));
    scenario.endpoint = Some("https://runtime.example".to_owned());
    scenario.logging = true;
    scenario
}

fn logging_case() {
    pre_call_snapshot();
    post_call_and_quiet_run();
}

fn pre_call_snapshot() {
    let run = super::fixture::run(&logging_scenario());
    run.result.as_ref().expect("the status lookup succeeds");
    assert_eq!(run.snapshots.len(), 2, "pre_call and post_call ran");
    let url = format!("https://runtime.example/async-invoke/{ENCODED_ARN}");
    let pre = additional_args(&run.snapshots[0]);
    assert_eq!(text(field(pre, "api_base")), url.as_str());
    assert_logged_headers(pre);
    let request_str = format!(
        "\n\nGET Request Sent from LiteLLM:\ncurl -X GET \\\n{url} \\\n-H 'Content-Type: application/json' -H 'X-Amz-Date: 20231114T221320Z' -H 'X-Amz-Security-Token: se****en' -H 'Authorization: AW****32'\n"
    );
    assert_eq!(text(field(pre, "request_str")), request_str.as_str());
    assert_eq!(field(pre, "complete_input_dict"), &complete_input());
    assert_eq!(text(field(&run.snapshots[0], "input")), INVOCATION_ARN);
}

fn assert_logged_headers(pre: &Json) {
    let headers = object(field(pre, "headers"));
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "Authorization",
            "Content-Type",
            "X-Amz-Date",
            "X-Amz-Security-Token"
        ]
    );
    assert_eq!(
        headers.get("Content-Type"),
        Some(&Json::Text("application/json".to_owned()))
    );
    assert_eq!(
        headers.get("X-Amz-Date"),
        Some(&Json::Text("20231114T221320Z".to_owned()))
    );
    // the callback snapshot scrubs a sensitive header key to B244's marker, so only the curl view shows the mask
    assert_eq!(
        headers.get("X-Amz-Security-Token"),
        Some(&Json::Text(REDACTED.to_owned()))
    );
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text(REDACTED.to_owned()))
    );
}

fn post_call_and_quiet_run() {
    let run = super::fixture::run(&logging_scenario());
    run.result.as_ref().expect("the status lookup succeeds");
    assert_eq!(
        text(field(&run.snapshots[1], "original_response")),
        r#"{"status":"Completed"}"#
    );
    assert_eq!(text(field(&run.snapshots[1], "input")), INVOCATION_ARN);
    assert_eq!(
        field(additional_args(&run.snapshots[1]), "complete_input_dict"),
        &complete_input()
    );
    let mut quiet_scenario = logging_scenario();
    quiet_scenario.logging = false;
    let quiet = super::fixture::run(&quiet_scenario);
    assert!(
        quiet.snapshots.is_empty(),
        "no logging object means no masking"
    );
    assert_eq!(quiet.requests, run.requests);
}

#[test]
fn embedding_jobs_credential_identity_isolation() {
    let mut first = Scenario::new(Reply::ok("{}"));
    first.endpoint = Some("https://first.example".to_owned());
    first.region = "eu-west-1".to_owned();
    first.credentials = credentials_with("AKIAFIRSTEXAMPLE", "first-secret", None);
    let first_run = super::fixture::run(&first);
    let mut second = Scenario::new(Reply::ok("{}"));
    second.endpoint = Some("https://second.example".to_owned());
    second.region = "us-east-2".to_owned();
    second
        .environment
        .insert("AWS_REGION".to_owned(), "us-east-2".to_owned());
    second.credentials = credentials_with("AKIASECONDEXAMPLE", "second-secret", None);
    let second_run = super::fixture::run(&second);
    let first_request = first_run.requests.first().expect("first request sent");
    let second_request = second_run.requests.first().expect("second request sent");
    assert_ne!(first_request.url, second_request.url);
    assert!(header(first_request, "Authorization").contains("Credential=AKIAFIRSTEXAMPLE/"));
    assert!(header(second_request, "Authorization").contains("Credential=AKIASECONDEXAMPLE/"));
    assert!(!header(second_request, "Authorization").contains("AKIAFIRSTEXAMPLE"));
    assert!(!header(first_request, "Authorization").contains("AKIASECONDEXAMPLE"));
    assert_eq!(first_run.resolved.len(), 1);
    assert_eq!(second_run.resolved.len(), 1);
    assert_eq!(first_run.resolved[0].1, "eu-west-1");
    assert_eq!(second_run.resolved[0].1, "us-east-2");
    let repeated = super::fixture::run(&first);
    assert_eq!(repeated.requests, first_run.requests);
}

#[test]
fn embedding_jobs_secret_safe_failure() {
    let reply_body = format!("rejected {ACCESS_KEY}, {SECRET_KEY} and {SESSION_TOKEN}");
    let mut scenario = Scenario::new(Reply::status(500, &reply_body));
    scenario.logging = true;
    let run = super::fixture::run(&scenario);
    let error = run.result.expect_err("a 500 fails");
    assert_eq!(
        error.to_string(),
        "Failed to get async invoke status: 500 - rejected [REDACTED], [REDACTED] and [REDACTED]"
    );
    let post = field(&run.snapshots[1], "original_response");
    assert_eq!(
        post,
        &Json::Text("rejected [REDACTED], [REDACTED] and [REDACTED]".to_owned())
    );
    assert!(!error.to_string().contains(ACCESS_KEY));
    let debug = format!("{error:?}");
    assert!(debug.contains("HttpFailure"));
    assert!(debug.contains("[REDACTED provider reply]"));
    assert!(!debug.contains(SECRET_KEY));
    assert!(!debug.contains(SESSION_TOKEN));
    let EmbeddingJobsError::HttpFailure { body, .. } = &error else {
        panic!("the failure is an HTTP status failure");
    };
    assert_eq!(body.0, "rejected [REDACTED], [REDACTED] and [REDACTED]");

    assert_request_debug_safe(run.requests.first().expect("the request was sent"));
    let failed = super::fixture::run(&Scenario::new(Reply::Failed));
    let transport = failed.result.expect_err("the transport fails");
    assert!(!format!("{transport:?}").contains(SECRET_KEY));
    assert!(!transport.to_string().contains(SECRET_KEY));
}

fn assert_request_debug_safe(request: &StatusWireRequest) {
    let request_debug = format!("{request:?}");
    assert!(request_debug.contains("header_names"));
    assert!(request_debug.contains("Content-Type"));
    assert!(!request_debug.contains(SECRET_KEY));
    assert!(!request_debug.contains(SESSION_TOKEN));
    assert!(!request_debug.contains("AWS4-HMAC-SHA256"));
}

#[rstest]
#[case::null("null", true)]
#[case::wrong_type("42", false)]
fn endpoint_parameter_types(#[case] value: &str, #[case] accepted: bool) {
    let mut scenario = Scenario::new(Reply::ok("{}"));
    scenario.optional_params.insert(
        "aws_bedrock_runtime_endpoint".to_owned(),
        RawValue::from_string(value.to_owned()).unwrap(),
    );
    let run = super::fixture::run(&scenario);
    if accepted {
        run.result.unwrap();
        assert_eq!(
            run.requests[0].url,
            format!("https://bedrock-runtime.us-west-2.amazonaws.com/async-invoke/{ENCODED_ARN}")
        );
    } else {
        assert!(matches!(
            run.result,
            Err(EmbeddingJobsError::Endpoint { .. })
        ));
        assert!(run.requests.is_empty());
    }
}

#[test]
fn bearer_parameters_do_not_skip_signing() {
    let mut scenario = Scenario::new(Reply::ok("{}"));
    scenario.environment.insert(
        "AWS_BEARER_TOKEN_BEDROCK".to_owned(),
        "unused-bearer".to_owned(),
    );
    scenario.optional_params.insert(
        "api_key".to_owned(),
        RawValue::from_string("\"unused-api-key\"".to_owned()).unwrap(),
    );
    let run = super::fixture::run(&scenario);
    run.result.unwrap();
    assert_eq!(run.resolved.len(), 1);
    assert_signed_headers(&run.requests[0]);
}

#[test]
fn status_timeout_is_not_converted() {
    let run = super::fixture::run(&Scenario::new(Reply::Timeout));
    assert!(
        matches!(run.result, Err(EmbeddingJobsError::Transport(TransferError::Timeout { elapsed })) if elapsed == Duration::from_secs(7))
    );
}

#[test]
fn empty_credentials_do_not_change_failure_text() {
    let mut scenario = Scenario::new(Reply::status(500, "plain failure"));
    scenario.credentials = credentials_with("", "", Some(""));
    let run = super::fixture::run(&scenario);
    assert_eq!(
        run.result.unwrap_err().to_string(),
        "Failed to get async invoke status: 500 - plain failure"
    );
}
