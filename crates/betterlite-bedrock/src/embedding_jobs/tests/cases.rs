use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::{EmbeddingJobsError, StatusWireRequest};
use super::fixture::{
    ENCODED_ARN, INVOCATION_ARN, Reply, SECRET_KEY, SESSION_TOKEN, Scenario, credentials_with,
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

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    value
        .as_object()
        .and_then(|object| object.get(name))
        .expect("the fixture field exists")
}

fn additional_args(snapshot: &Json) -> &Json {
    field(snapshot, "additional_args")
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
    let names: Vec<&str> = request
        .headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        names,
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
    // credential loading follows the environment region; the signature follows the method argument
    assert_eq!(
        run.resolved.first().map(|(_, region)| region.as_str()),
        Some("eu-west-1")
    );
    assert_eq!(run.factory_calls, vec![("bedrock".to_owned(), None)]);

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
    let body = r#"{"status":"Failed","metadata":null,"unknown":{"s3":"s3://bucket/output"},"nested":[1,true,null]}"#;
    let run = super::fixture::run(&Scenario::new(Reply::ok(body)));
    let value = run
        .result
        .expect("a Failed job in a 200 body is a lookup success");
    assert_eq!(
        value,
        serde_json::from_str::<Json>(body).expect("the fixture body is JSON")
    );
    assert_eq!(run.requests.len(), 1);
    assert_eq!(field(&value, "status"), &Json::Text("Failed".to_owned()));
    assert_eq!(
        field(field(&value, "unknown"), "s3"),
        &Json::Text("s3://bucket/output".to_owned())
    );

    let completed = r#"{"status":"Completed","output":{"s3Uri":"s3://bucket/out"}}"#;
    let completed_run = super::fixture::run(&Scenario::new(Reply::ok(completed)));
    assert_eq!(
        completed_run
            .result
            .expect("a Completed job returns its body"),
        serde_json::from_str::<Json>(completed).expect("the fixture body is JSON")
    );

    let in_progress = r#"{"status":"InProgress","unknown":{"s3":"s3://bucket/x"}}"#;
    assert_eq!(
        super::fixture::run(&Scenario::new(Reply::ok(in_progress)))
            .result
            .expect("an InProgress job returns its body"),
        serde_json::from_str::<Json>(in_progress).expect("the fixture body is JSON")
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
        let error = super::fixture::run(&Scenario::new(Reply::status(status, "created")))
            .result
            .expect_err("every non-200 status fails");
        assert_eq!(
            error.to_string(),
            format!("Failed to get async invoke status: {status} - created")
        );
    }

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
        Err(EmbeddingJobsError::Transport(
            crate::bedrock_embedding_io::transport::TransferError::Failed(TransportError::Closed)
        ))
    ));

    let mut unsigned = Scenario::new(Reply::ok("{}"));
    unsigned.endpoint = Some("://not-a-url".to_owned());
    assert!(matches!(
        super::fixture::run(&unsigned).result,
        Err(EmbeddingJobsError::Signing(_))
    ));
}

fn logging_case() {
    let mut scenario = Scenario::new(Reply::ok(r#"{"status":"Completed"}"#));
    scenario.endpoint = Some("https://runtime.example".to_owned());
    scenario.logging = true;
    let run = super::fixture::run(&scenario);
    run.result.as_ref().expect("the status lookup succeeds");
    assert_eq!(run.snapshots.len(), 2, "pre_call and post_call ran");
    let url = format!("https://runtime.example/async-invoke/{ENCODED_ARN}");
    let pre = additional_args(&run.snapshots[0]);
    assert_eq!(field(pre, "api_base").as_str(), Some(url.as_str()));
    let headers = field(pre, "headers")
        .as_object()
        .expect("the pre_call headers are an object");
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "Content-Type",
            "X-Amz-Date",
            "X-Amz-Security-Token",
            "Authorization"
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
    assert_eq!(
        headers.get("X-Amz-Security-Token"),
        Some(&Json::Text("se****en".to_owned()))
    );
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text("AW****32".to_owned()))
    );
    let request_str = format!(
        "\n\nGET Request Sent from LiteLLM:\ncurl -X GET \\\n{url} \\\n-H 'Content-Type: application/json' -H 'X-Amz-Date: 20231114T221320Z' -H 'X-Amz-Security-Token: se****en' -H 'Authorization: AW****32'\n"
    );
    assert_eq!(
        field(pre, "request_str").as_str(),
        Some(request_str.as_str())
    );
    assert_eq!(
        field(pre, "complete_input_dict"),
        &serde_json::from_str::<Json>(&format!("{{\"invocation_arn\":\"{INVOCATION_ARN}\"}}"))
            .expect("the fixture object is JSON")
    );
    assert_eq!(
        field(&run.snapshots[0], "input").as_str(),
        Some(INVOCATION_ARN)
    );

    assert_eq!(
        field(&run.snapshots[1], "original_response").as_str(),
        Some(r#"{"status":"Completed"}"#)
    );
    assert_eq!(
        field(&run.snapshots[1], "input").as_str(),
        Some(INVOCATION_ARN)
    );
    assert_eq!(
        field(additional_args(&run.snapshots[1]), "complete_input_dict"),
        &serde_json::from_str::<Json>(&format!("{{\"invocation_arn\":\"{INVOCATION_ARN}\"}}"))
            .expect("the fixture object is JSON")
    );

    scenario.logging = false;
    let quiet = super::fixture::run(&scenario);
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
    let reply_body = format!("rejected {SECRET_KEY} and {SESSION_TOKEN}");
    let run = super::fixture::run(&Scenario::new(Reply::status(500, &reply_body)));
    let error = run.result.expect_err("a 500 fails");
    assert_eq!(
        error.to_string(),
        format!("Failed to get async invoke status: 500 - {reply_body}")
    );
    assert!(!format!("{error:?}").contains(SECRET_KEY));
    assert!(!format!("{error:?}").contains(SESSION_TOKEN));
    let Ok(EmbeddingJobsError::HttpFailure { body, .. }) = &error else {
        panic!("the failure is an HTTP status failure");
    };
    assert_eq!(body.as_str(), reply_body);

    let request = run.requests.first().expect("the request was sent");
    let debug = format!("{request:?}");
    assert!(!debug.contains(SECRET_KEY));
    assert!(!debug.contains(SESSION_TOKEN));
    assert!(!debug.contains("AWS4-HMAC-SHA256"));

    let failed = super::fixture::run(&Scenario::new(Reply::Failed));
    let transport = failed.result.expect_err("the transport fails");
    assert!(!format!("{transport:?}").contains(SECRET_KEY));
    assert!(!transport.to_string().contains(SECRET_KEY));
}
