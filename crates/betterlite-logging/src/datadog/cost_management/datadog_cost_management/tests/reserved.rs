use indexmap::IndexMap;
use rstest::rstest;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::tags::extract_tags;

fn tags(allow: &[&str], payload: &support::Raw) -> IndexMap<String, String> {
    let settings = support::settings(&support::init(allow));
    extract_tags(
        &settings.environment,
        &settings.cost_tag_keys,
        &support::payload(&payload.0),
    )
}

#[rstest]
#[case::env("env", Some("prod"))]
#[case::service("service", Some("gateway"))]
#[case::host("host", Some("host-1"))]
#[case::pod_name("pod_name", Some("pod-1"))]
#[case::provider("provider", Some("openai"))]
#[case::model("model", Some("gpt-4o"))]
#[case::model_id("model_id", Some("router-1"))]
#[case::team("team", None)]
#[case::user("user", None)]
#[case::model_group("model_group", Some("production"))]
fn reserved_keys_are_dropped_even_when_allowlisted(
    #[case] key: &str,
    #[case] trusted: Option<&str>,
) {
    let payload = support::default_payload()
        .set(
            "/request_tags",
            support::list(vec![support::text(&format!("{key}:attacker"))]),
        )
        .set(
            "/metadata/spend_logs_metadata",
            support::object(vec![(key, support::text("attacker"))]),
        );
    let (found, logs) = support::logged(|| tags(&[key], &payload));
    assert_eq!(support::tag(&found, key), trusted);
    assert!(logs.contains("reserved"));
}

#[test]
fn reserved_keys_keep_the_trusted_user_team_and_model_group_tags() {
    let payload = support::default_payload()
        .set(
            "/metadata/user_api_key_alias",
            support::text("trusted-user"),
        )
        .set(
            "/metadata/user_api_key_team_alias",
            support::text("trusted-team"),
        )
        .set("/model_group", support::text("trusted-group"))
        .set(
            "/request_tags",
            support::list(vec![
                support::text("user:attacker"),
                support::text("team:attacker"),
                support::text("model_group:attacker"),
            ]),
        )
        .set(
            "/metadata/requester_metadata",
            support::object(vec![
                ("user", support::text("attacker")),
                ("team", support::text("attacker")),
                ("model_group", support::text("attacker")),
            ]),
        );
    let found = tags(&["user", "team", "model_group"], &payload);
    assert_eq!(support::tag(&found, "user"), Some("trusted-user"));
    assert_eq!(support::tag(&found, "team"), Some("trusted-team"));
    assert_eq!(support::tag(&found, "model_group"), Some("trusted-group"));
}
