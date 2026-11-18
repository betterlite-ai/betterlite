use betterlite_models::decisions::Json as Decisions;
use betterlite_prompts::arguments::json::Object;

use super::super::fixture::{Reply, fixture_probe, run};
use super::super::support::{
    argument_fields, decisions_object, decisions_value, fields, phases, titan_body,
};

const SIGNED: &str = "SignedHeaders=content-type;host;x-amz-date;x-amz-security-token";

pub(super) fn signing_auth_isolation() {
    every_request_carries_its_own_signature();
    custom_headers_merge_before_signing();
    bearer_and_authorization_precedence();
}

fn every_request_carries_its_own_signature() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.input = vec![String::from("one"), String::from("two")];
    outcome_probe.replies = vec![
        Reply::ok(&titan_body("[1.0]", 1)),
        Reply::ok(&titan_body("[2.0]", 1)),
    ];
    let outcome = run(&outcome_probe);
    assert_eq!(outcome.factory_requests.len(), 2);
    let first = &outcome.factory_requests[0];
    let second = &outcome.factory_requests[1];
    assert_ne!(
        first.headers["Authorization"],
        second.headers["Authorization"]
    );
    assert_eq!(first.headers["X-Amz-Date"], "20231114T221320Z");
    assert_eq!(
        first.headers["Content-Length"],
        first.body.len().to_string()
    );
    assert_eq!(
        phases(&outcome.snapshots),
        [
            "pre_api_call",
            "post_api_call",
            "pre_api_call",
            "post_api_call"
        ]
    );
    let sent: Object = serde_json::from_slice(&first.body).expect("the first body is an object");
    assert_eq!(
        fields(&outcome.snapshots[0]).get("input"),
        Some(&decisions_object(&sent))
    );
    let arguments = argument_fields(&outcome.snapshots[0]);
    assert_eq!(
        arguments.get("api_base"),
        Some(&Decisions::Text(first.url.clone()))
    );
    assert_eq!(
        arguments.get("complete_input_dict"),
        Some(&decisions_object(&sent))
    );
    let reply = snapshot_response(&outcome.snapshots[1]);
    assert_eq!(reply, parsed(&titan_body("[1.0]", 1)));
}

fn custom_headers_merge_before_signing() {
    let mut outcome_probe = fixture_probe();
    outcome_probe.extra_headers = Some(vec![
        (
            String::from("Content-Type"),
            String::from("application/json; charset=utf-8"),
        ),
        (String::from("X-Custom"), String::from("abc")),
    ]);
    outcome_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let outcome = run(&outcome_probe);
    let headers = outcome.headers();
    assert_eq!(headers["Content-Type"], "application/json; charset=utf-8");
    assert_eq!(headers["X-Custom"], "abc");
    assert!(headers["Authorization"].contains(SIGNED));
    assert!(headers["Authorization"].starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20231114/us-east-1/bedrock/aws4_request"
    ));
    assert_eq!(headers["X-Amz-Security-Token"], "session-token");
    assert!(!headers.contains_key("Host"));
}

fn bearer_and_authorization_precedence() {
    let mut bearer_probe = fixture_probe();
    bearer_probe.api_key = Some(String::from("caller-token"));
    bearer_probe.bearer = Some(String::from("configured-token"));
    bearer_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let bearer = run(&bearer_probe);
    assert_eq!(bearer.headers()["Authorization"], "Bearer caller-token");
    assert!(!bearer.headers().contains_key("X-Amz-Date"));
    assert!(bearer.regions.is_empty());
    let mut overridden_probe = fixture_probe();
    overridden_probe.extra_headers = Some(vec![(
        String::from("Authorization"),
        String::from("Basic abc"),
    )]);
    overridden_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let overridden = run(&overridden_probe);
    assert_eq!(overridden.headers()["Authorization"], "Basic abc");
    let mut forged_probe = fixture_probe();
    forged_probe.extra_headers = Some(vec![(
        String::from("Authorization"),
        String::from("AWS4-HMAC-SHA256 forged"),
    )]);
    forged_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let forged = run(&forged_probe);
    assert!(
        forged.headers()["Authorization"]
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/")
    );
    let mut empty_probe = fixture_probe();
    empty_probe.api_key = Some(String::new());
    empty_probe.bearer = Some(String::new());
    empty_probe.replies = vec![Reply::ok(&titan_body("[1.0]", 1))];
    let empty = run(&empty_probe);
    assert!(empty.headers()["Authorization"].starts_with("AWS4-HMAC-SHA256 Credential="));
    assert_eq!(empty.regions, ["us-east-1"]);
}

fn snapshot_response(snapshot: &Decisions) -> Decisions {
    match fields(snapshot).get("original_response") {
        Some(Decisions::Text(text)) => serde_json::from_str(text).expect("the logged reply parses"),
        Some(other) => other.clone(),
        None => panic!("the post call logs its reply"),
    }
}

fn parsed(text: &str) -> Decisions {
    let value: betterlite_prompts::arguments::json::Json =
        serde_json::from_str(text).expect("the expected reply parses");
    decisions_value(&value)
}
