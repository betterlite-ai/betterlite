use std::collections::BTreeMap;

use rstest::rstest;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::tags::extract_tags;

#[rstest]
#[case::punctuation("P&T", "p_t")]
#[case::hyphen("CTO-B2B", "cto-b2b")]
#[case::spaces_and_marks("  Team  &  Key!!  ", "team_key")]
#[case::kept_separators("a:b.c/d_e", "a:b.c/d_e")]
fn normalization_applies_to_user_team_and_custom_tags(#[case] value: &str, #[case] expected: &str) {
    let settings = support::settings(&support::init(&["capability"]));
    let payload = support::default_payload()
        .set("/metadata/user_api_key_alias", support::text(value))
        .set("/metadata/user_api_key_team_alias", support::text(value))
        .set(
            "/request_tags",
            support::list(vec![support::text(&format!("capability:{value}"))]),
        )
        .build();
    let tags = extract_tags(
        &settings.environment,
        &settings.cost_tag_keys,
        payload.as_ref(),
    );
    assert_eq!(support::tag(&tags, "user"), Some(expected));
    assert_eq!(support::tag(&tags, "team"), Some(expected));
    assert_eq!(support::tag(&tags, "capability"), Some(expected));
}

#[test]
fn normalization_leaves_base_tags_and_focus_dimensions_alone() {
    let settings = support::settings_with(
        &support::init(&[]),
        &BTreeMap::from([
            ("DD_API_KEY".to_owned(), "api-key".to_owned()),
            ("DD_APP_KEY".to_owned(), "app-key".to_owned()),
            ("DD_ENV".to_owned(), "Prod & QA".to_owned()),
            ("DD_SERVICE".to_owned(), "Billing API".to_owned()),
            ("HOSTNAME".to_owned(), "Host.1".to_owned()),
            ("POD_NAME".to_owned(), "Pod_1".to_owned()),
        ]),
    );
    let payload = support::default_payload()
        .set("/custom_llm_provider", support::text("OpenAI:V1"))
        .set("/model", support::text("GPT/4o"))
        .set("/model_id", support::text("Router ID"))
        .build();
    let tags = extract_tags(
        &settings.environment,
        &settings.cost_tag_keys,
        payload.as_ref(),
    );
    assert_eq!(support::tag(&tags, "env"), Some("Prod & QA"));
    assert_eq!(support::tag(&tags, "service"), Some("Billing API"));
    assert_eq!(support::tag(&tags, "host"), Some("Host.1"));
    assert_eq!(support::tag(&tags, "pod_name"), Some("Pod_1"));
    assert_eq!(support::tag(&tags, "provider"), Some("OpenAI:V1"));
    assert_eq!(support::tag(&tags, "model"), Some("GPT/4o"));
    assert_eq!(support::tag(&tags, "model_id"), Some("Router ID"));
}
