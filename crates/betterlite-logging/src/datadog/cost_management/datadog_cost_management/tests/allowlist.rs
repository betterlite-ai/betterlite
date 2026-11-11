use indexmap::IndexMap;

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

#[test]
fn allowlist_gates_request_tags_and_nested_metadata() {
    let payload = support::default_payload()
        .set(
            "/request_tags",
            support::list(vec![
                support::text("capability:Chat Bot"),
                support::text("tier:Gold"),
                support::text("secret:nope"),
            ]),
        )
        .set(
            "/metadata/spend_logs_metadata",
            support::object(vec![
                ("capability", support::text("Chat Bot")),
                ("secret", support::text("x")),
            ]),
        )
        .set(
            "/metadata/requester_metadata",
            support::object(vec![
                ("tier", support::text("Gold")),
                ("secret", support::text("x")),
            ]),
        );
    let found = tags(&["capability", "tier"], &payload);
    assert_eq!(support::tag(&found, "capability"), Some("chat_bot"));
    assert_eq!(support::tag(&found, "tier"), Some("gold"));
    assert_eq!(support::tag(&found, "secret"), None);
}

#[test]
fn allowlist_carries_nested_metadata_tags() {
    let payload = support::default_payload()
        .set(
            "/metadata/spend_logs_metadata",
            support::object(vec![
                ("platform", support::text("web")),
                ("ignored", support::text("x")),
            ]),
        )
        .set(
            "/metadata/requester_metadata",
            support::object(vec![("region", support::text("eu"))]),
        );
    let found = tags(&["platform", "region"], &payload);
    assert_eq!(support::tag(&found, "platform"), Some("web"));
    assert_eq!(support::tag(&found, "region"), Some("eu"));
    assert_eq!(support::tag(&found, "ignored"), None);
}

#[test]
fn allowlist_carries_a_declared_metadata_field() {
    let payload = support::default_payload().set(
        "/metadata/user_api_key_user_email",
        support::text("dev@example.com"),
    );
    let found = tags(&["user_api_key_user_email"], &payload);
    assert_eq!(
        support::tag(&found, "user_api_key_user_email"),
        Some("dev_example.com")
    );
}

#[test]
fn allowlist_empty_keeps_request_and_metadata_tags_out() {
    let payload = support::default_payload()
        .set(
            "/request_tags",
            support::list(vec![support::text("capability:Chat")]),
        )
        .set(
            "/metadata/spend_logs_metadata",
            support::object(vec![("capability", support::text("Chat"))]),
        );
    let found = tags(&[], &payload);
    assert_eq!(support::tag(&found, "capability"), None);
}
