use std::collections::BTreeMap;

use super::{
    IDENTIFIER, Reply, Scripted, VERSION, expected, field_of, guardrail, guardrail_with_checks,
    has_field, input_scan, message, run, text,
};

#[tokio::test]
async fn a_checks_guardrail_routes_to_the_checks_endpoint() {
    let guardrail = guardrail_with_checks();
    let messages = vec![message("user", "hello")];
    let transport = Scripted::new(vec![Reply::ok(
        r#"{
            "results": {
                "contentFilter": { "results": [] },
                "promptAttack": { "results": [] },
                "sensitiveInformation": { "results": [], "truncated": false }
            }
        }"#,
    )]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail, &mut data, input_scan(&messages), &transport).await;
    drop(outcome);
    let urls = transport.urls();
    assert_eq!(urls.len(), 1);
    assert!(urls[0].ends_with("/guardrail-checks/invoke"));
    let bodies = transport.bodies();
    assert!(!has_field(&bodies[0], "content"));
    assert_eq!(
        field_of(&bodies[0], "checks"),
        expected(r#"{ "contentFilter": {} }"#)
    );
}

#[tokio::test]
async fn an_apply_guardrail_posts_the_converted_content_to_the_apply_path() {
    let messages = vec![message("user", "hello")];
    let transport = Scripted::new(vec![Reply::ok(r#"{ "action": "NONE" }"#)]);
    let mut data = BTreeMap::new();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    let response = outcome.expect("the probe passes");
    assert_eq!(response.action.as_deref(), Some("NONE"));
    let urls = transport.urls();
    assert_eq!(urls.len(), 1);
    let apply_path = format!("/guardrail/{IDENTIFIER}/version/{VERSION}/apply");
    assert!(urls[0].starts_with("https://"));
    assert!(urls[0].ends_with(&apply_path));
    let bodies = transport.bodies();
    assert_eq!(field_of(&bodies[0], "source"), text("INPUT"));
    assert_eq!(
        field_of(&bodies[0], "content"),
        expected(r#"[{ "text": { "text": "hello" } }]"#)
    );
}
