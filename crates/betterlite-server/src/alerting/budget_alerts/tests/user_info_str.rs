#![cfg(test)]

use std::collections::BTreeMap;

use crate::failure_hook::ports::EntityType;

use super::events::get_user_info_str;
use super::support::{call_info, decimal};

#[test]
fn only_the_present_fields_are_rendered() {
    let info = call_info(decimal("100"));

    assert_eq!(
        get_user_info_str(&info),
        "*spend:* `100`\n*event_group:* `key`\n"
    );
}

#[test]
fn the_key_token_is_not_rendered() {
    let mut info = call_info(decimal("100"));
    info.token = Some("hashed-key".to_owned());

    let text = get_user_info_str(&info);

    assert_eq!(text, "*spend:* `100`\n*event_group:* `key`\n");
    assert!(!text.contains("hashed-key"));
}

#[test]
fn a_missing_field_is_skipped() {
    let mut info = call_info(decimal("100"));
    info.team_id = Some("team-1".to_owned());

    let text = get_user_info_str(&info);

    assert!(!text.contains("user_id"));
    assert!(text.contains("*team_id:* `team-1`\n"));
}

fn all_fields() -> CallInfo {
    let mut info = call_info(decimal("300"));
    info.max_budget = Some(decimal("1000"));
    info.soft_budget = Some(decimal("500"));
    info.token = Some("hashed-key".to_owned());
    info.customer_id = Some("cust-1".to_owned());
    info.user_id = Some("user-1".to_owned());
    info.team_id = Some("team-1".to_owned());
    info.team_alias = Some("Team One".to_owned());
    info.organization_id = Some("org-1".to_owned());
    info.user_email = Some("u@example.com".to_owned());
    info.key_alias = Some("key-1".to_owned());
    info.projected_exceeded_date = Some("2026-02-01".to_owned());
    info.projected_spend = Some(decimal("900.5"));
    info.event_group = EntityType::Team;
    info.alert_emails = Some(vec!["a@example.com".to_owned(), "b@example.com".to_owned()]);
    info.max_budget_alert_emails = Some(BTreeMap::from([
        ("50".to_owned(), vec!["a@example.com".to_owned()]),
        (
            "75".to_owned(),
            vec!["a@example.com".to_owned(), "b@example.com".to_owned()],
        ),
    ]));
    info
}

#[test]
fn every_field_renders_in_the_litellm_order() {
    let info = all_fields();

    assert_eq!(
        get_user_info_str(&info),
        concat!(
            "*spend:* `300`\n",
            "*max_budget:* `1000`\n",
            "*soft_budget:* `500`\n",
            "*customer_id:* `cust-1`\n",
            "*user_id:* `user-1`\n",
            "*team_id:* `team-1`\n",
            "*team_alias:* `Team One`\n",
            "*organization_id:* `org-1`\n",
            "*user_email:* `u@example.com`\n",
            "*key_alias:* `key-1`\n",
            "*projected_exceeded_date:* `2026-02-01`\n",
            "*projected_spend:* `900.5`\n",
            "*event_group:* `team`\n",
            "*alert_emails:* `[\"a@example.com\", \"b@example.com\"]`\n",
            "*max_budget_alert_emails:* `{\"50\": [\"a@example.com\"], \"75\": [\"a@example.com\", \"b@example.com\"]}`\n",
        )
    );
}

#[test]
fn the_entity_group_renders_its_value() {
    let mut info = call_info(decimal("1"));
    info.event_group = EntityType::TeamMember;

    assert!(get_user_info_str(&info).contains("*event_group:* `team_member`\n"));
}
