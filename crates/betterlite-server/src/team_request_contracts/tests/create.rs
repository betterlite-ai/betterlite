use std::collections::BTreeMap;

use betterlite_db::team_rows::member::MemberRole;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{object, parse};
use crate::key_fields::values::{Amount, IndexPermission};
use crate::team_request_contracts::bulk_members::TeamMemberRef;
use crate::team_request_contracts::create::{NewTeamFields, NewTeamRequest};
use crate::team_request_contracts::shared::RequestProjection as _;
use crate::team_request_contracts::team_list::TeamListEntry;
use crate::team_request_contracts::values::{PositiveInt, ThroughputLimitType};

const CREATE_FIELDS: &str = r#"{
    "team_alias": "alias", "team_id": "team-1", "organization_id": "org",
    "admins": ["a"], "members": ["m1"],
    "members_with_roles": [{"user_id": "u", "role": "admin"}],
    "team_member_permissions": ["/key/info"], "metadata": {"k": "v"},
    "tpm_limit": 10, "rpm_limit": 20, "tpd_limit": 30,
    "max_budget": 10.5, "soft_budget": 5, "budget_duration": "30d",
    "budget_limits": [{"budget_duration": "1d", "max_budget": 2}],
    "models": ["gpt-4o"], "blocked": true, "router_settings": {"weights": {"g": {"d": 1}}},
    "access_group_ids": ["ag"], "default_team_member_models": ["gpt-4o"],
    "model_aliases": {"m": "target"},
    "model_max_budget": {"gpt-4o": {"max_budget": 10, "budget_duration": "1d"}},
    "tags": ["t"], "guardrails": ["g"], "policies": ["p"], "prompts": ["pr"],
    "object_permission": {"models": ["gpt-4o"]},
    "allowed_passthrough_routes": ["/v1/models"], "disable_global_guardrails": true,
    "secret_manager_settings": {"aws": {}}, "model_rpm_limit": {"gpt-4o": 5},
    "rpm_limit_type": "guaranteed_throughput", "tpm_limit_type": "best_effort_throughput",
    "model_tpm_limit": {"gpt-4o": 7}, "default_estimated_output_tokens": 9,
    "default_estimated_output_tokens_per_model": {"gpt-4o": 11},
    "mcp_rpm_limit": {"mcp": 3}, "team_member_budget": 12.5,
    "team_member_rpm_limit": 13, "team_member_tpm_limit": 14,
    "team_member_key_duration": "30d", "team_member_budget_duration": "1mo",
    "allowed_vector_store_indexes": [{"index_name": "i", "index_permissions": ["read", "write"]}],
    "enforced_batch_output_expires_after": {"h": 24},
    "enforced_file_expires_after": {"h": 48}
}"#;

#[test]
fn team_create_all_fields() {
    let request = NewTeamRequest::from_value(&parse(CREATE_FIELDS)).unwrap();
    let team = &request.team;
    assert_eq!(team.team_alias.as_deref(), Some("alias"));
    assert_eq!(team.team_id.as_deref(), Some("team-1"));
    assert_eq!(team.organization_id.as_deref(), Some("org"));
    assert_eq!(team.admins, vec!["a".to_owned()]);
    assert_eq!(team.members, vec!["m1".to_owned()]);
    assert_eq!(team.members_with_roles.len(), 1);
    assert_eq!(
        team.members_with_roles[0].base.user_id.as_deref(),
        Some("u")
    );
    assert_eq!(team.members_with_roles[0].role, MemberRole::Admin);
    assert_eq!(
        team.team_member_permissions,
        Some(vec!["/key/info".to_owned()])
    );
    assert_eq!(
        team.metadata.as_ref().and_then(|values| values.get("k")),
        Some(&parse(r#""v""#))
    );
    assert_eq!(team.tpm_limit, Some(10));
    assert_eq!(team.rpm_limit, Some(20));
}

#[test]
fn team_create_team_base_fields() {
    let request = NewTeamRequest::from_value(&parse(CREATE_FIELDS)).unwrap();
    let team = &request.team;
    assert_eq!(team.tpd_limit, Some(30));
    assert_eq!(team.max_budget, Some(Decimal::new(105, 1)));
    assert_eq!(team.soft_budget, Some(Decimal::from(5_i64)));
    assert_eq!(team.budget_duration.as_deref(), Some("30d"));
    assert_eq!(team.budget_limits.as_ref().map(Vec::len), Some(1));
    assert_eq!(team.models, vec!["gpt-4o".to_owned()]);
    assert!(team.blocked);
    assert!(team.router_settings.is_some());
    assert_eq!(team.access_group_ids, Some(vec!["ag".to_owned()]));
    assert_eq!(
        team.default_team_member_models,
        Some(vec!["gpt-4o".to_owned()])
    );
}

#[test]
fn team_create_extra_fields() {
    let request = NewTeamRequest::from_value(&parse(CREATE_FIELDS)).unwrap();
    let extra = &request.extra;
    assert_eq!(
        extra
            .model_aliases
            .as_ref()
            .and_then(|values| values.get("m")),
        Some(&parse(r#""target""#))
    );
    assert_eq!(
        extra
            .model_max_budget
            .as_ref()
            .and_then(|values| values.get("gpt-4o"))
            .and_then(|config| config.max_budget),
        Some(Amount(Decimal::from(10_i64)))
    );
    assert_eq!(extra.tags, Some(vec![parse(r#""t""#)]));
    assert_eq!(extra.guardrails, Some(vec!["g".to_owned()]));
    assert_eq!(extra.policies, Some(vec!["p".to_owned()]));
    assert_eq!(extra.prompts, Some(vec!["pr".to_owned()]));
    assert!(extra.object_permission.is_some());
    assert_eq!(
        extra.allowed_passthrough_routes,
        Some(vec![parse(r#""/v1/models""#)])
    );
    assert_eq!(extra.disable_global_guardrails, Some(true));
}

#[test]
fn team_create_extra_defaults_and_nulls() {
    let absent = NewTeamRequest::from_value(&parse(r#"{"team_id": "t"}"#)).unwrap();
    assert_eq!(absent.extra, NewTeamFields::default());
    let nulls = NewTeamRequest::from_value(&parse(
        r#"{"tags": null, "guardrails": null, "object_permission": null, "team_member_budget": null, "router_settings": null}"#,
    ))
    .unwrap();
    assert_eq!(nulls.extra.tags, None);
    assert_eq!(nulls.extra.object_permission, None);
    assert_eq!(nulls.extra.team_member_budget, None);
    assert_eq!(nulls.team.router_settings, None);
    assert_eq!(
        nulls.supplied().unwrap(),
        parse(
            r#"{"tags": null, "guardrails": null, "object_permission": null, "team_member_budget": null, "router_settings": null}"#
        )
    );
}

#[test]
fn team_create_extra_limit_fields() {
    let request = NewTeamRequest::from_value(&parse(CREATE_FIELDS)).unwrap();
    let extra = &request.extra;
    assert_eq!(
        extra.secret_manager_settings.as_ref().map(BTreeMap::len),
        Some(1)
    );
    assert_eq!(
        extra
            .model_rpm_limit
            .as_ref()
            .and_then(|values| values.get("gpt-4o")),
        Some(&5)
    );
    assert_eq!(extra.rpm_limit_type, Some(ThroughputLimitType::Guaranteed));
    assert_eq!(extra.tpm_limit_type, Some(ThroughputLimitType::BestEffort));
    assert_eq!(
        extra
            .model_tpm_limit
            .as_ref()
            .and_then(|values| values.get("gpt-4o")),
        Some(&7)
    );
    assert_eq!(extra.estimated_output_tokens, Some(PositiveInt(9)));
    assert_eq!(
        extra
            .per_model_output_tokens
            .as_ref()
            .and_then(|values| values.get("gpt-4o")),
        Some(&PositiveInt(11))
    );
}

#[test]
fn team_create_extra_budget_and_index_fields() {
    let request = NewTeamRequest::from_value(&parse(CREATE_FIELDS)).unwrap();
    let extra = &request.extra;
    assert_eq!(
        extra
            .mcp_rpm_limit
            .as_ref()
            .and_then(|values| values.get("mcp")),
        Some(&3)
    );
    assert_eq!(extra.team_member_budget, Some(Amount(Decimal::new(125, 1))));
    assert_eq!(extra.team_member_rpm_limit, Some(13));
    assert_eq!(extra.team_member_tpm_limit, Some(14));
    assert_eq!(extra.team_member_key_duration.as_deref(), Some("30d"));
    assert_eq!(extra.team_member_budget_duration.as_deref(), Some("1mo"));
    let indexes = extra.allowed_vector_store_indexes.as_ref().unwrap();
    assert_eq!(indexes[0].index_name, "i");
    assert_eq!(
        indexes[0].index_permissions,
        vec![IndexPermission::Read, IndexPermission::Write]
    );
    assert_eq!(
        extra
            .enforced_batch_expiry
            .as_ref()
            .and_then(|values| values.get("h")),
        Some(&parse("24"))
    );
    assert_eq!(
        extra
            .enforced_file_expires_after
            .as_ref()
            .and_then(|values| values.get("h")),
        Some(&parse("48"))
    );
}

#[test]
fn team_create_object_permission_projection() {
    let request = NewTeamRequest::from_value(&parse(CREATE_FIELDS)).unwrap();
    let resolved = request.resolved().unwrap();
    let permission = object(object(&resolved).get("object_permission").unwrap());
    assert_eq!(permission.len(), 12);
    assert_eq!(permission.get("models"), Some(&parse(r#"["gpt-4o"]"#)));
}

#[test]
fn create_blank_id_only() {
    let blank = NewTeamRequest::from_value(&parse(r#"{"team_id": "  "}"#)).unwrap();
    assert_eq!(blank.team.team_id, None);
    assert_eq!(blank.supplied().unwrap(), parse(r#"{"team_id": null}"#));
    let spaced = NewTeamRequest::from_value(&parse(r#"{"team_id": "\t\n"}"#)).unwrap();
    assert_eq!(spaced.team.team_id, None);
    let nonblank = NewTeamRequest::from_value(&parse(r#"{"team_id": " t "}"#)).unwrap();
    assert_eq!(nonblank.team.team_id.as_deref(), Some(" t "));
    assert_eq!(nonblank.supplied().unwrap(), parse(r#"{"team_id": " t "}"#));
    let absent = NewTeamRequest::from_value(&parse("{}")).unwrap();
    assert_eq!(absent.team.team_id, None);
    assert_eq!(absent.supplied().unwrap(), parse("{}"));
    let control =
        NewTeamRequest::from_value(&parse(r#"{"team_id": "\u001c\u001d\u001e\u001f"}"#)).unwrap();
    assert_eq!(control.team.team_id, None);
    assert_eq!(control.supplied().unwrap(), parse(r#"{"team_id": null}"#));
}

#[test]
fn non_object_documents_rejected() {
    let array = NewTeamRequest::from_value(&parse("[1]")).unwrap_err();
    assert_eq!(
        array.to_string(),
        "Input should be a valid dictionary or object"
    );
    let null = NewTeamRequest::from_value(&parse("null")).unwrap_err();
    assert_eq!(
        null.to_string(),
        "Input should be a valid dictionary or object"
    );
    let member = TeamMemberRef::from_value(&parse("[1]")).unwrap_err();
    assert_eq!(
        member.to_string(),
        "Input should be a valid dictionary or object"
    );
    let row = TeamListEntry::from_value(&parse("[1]")).unwrap_err();
    assert_eq!(
        row.to_string(),
        "Input should be a valid dictionary or object"
    );
}

#[rstest]
#[case::small("1", true)]
#[case::large("1024", true)]
#[case::zero("0", false)]
#[case::negative("-1", false)]
#[case::fraction("1.5", false)]
fn positive_output_estimates(#[case] value: &str, #[case] accepted: bool) {
    let body = format!(
        r#"{{"default_estimated_output_tokens": {value}, "default_estimated_output_tokens_per_model": {{"gpt-4o": {value}}}}}"#
    );
    let outcome = NewTeamRequest::from_value(&parse(&body));
    assert_eq!(outcome.is_ok(), accepted);
    if let Ok(request) = outcome {
        assert!(request.extra.estimated_output_tokens.is_some());
        assert!(request.extra.per_model_output_tokens.is_some());
    }
}

#[rstest]
#[case::guaranteed("guaranteed_throughput", Some(ThroughputLimitType::Guaranteed))]
#[case::best_effort("best_effort_throughput", Some(ThroughputLimitType::BestEffort))]
#[case::dynamic("dynamic", None)]
#[case::unknown("turbo", None)]
#[case::empty("", None)]
fn throughput_enum_values(#[case] text: &str, #[case] expected: Option<ThroughputLimitType>) {
    let body = format!(r#"{{"rpm_limit_type": "{text}", "tpm_limit_type": "{text}"}}"#);
    let outcome = NewTeamRequest::from_value(&parse(&body));
    match expected {
        Some(kind) => {
            let request = outcome.unwrap();
            assert_eq!(request.extra.rpm_limit_type, Some(kind));
            assert_eq!(request.extra.tpm_limit_type, Some(kind));
        }
        None => {
            outcome.unwrap_err();
        }
    }
}

#[rstest]
#[case::weights(r#"{"weights": {"g": {"d": 1}}}"#, true)]
#[case::null_weights(r#"{"weights": null}"#, true)]
#[case::other_key(r#"{"other": 1}"#, true)]
#[case::empty_group(r#"{"weights": {"g": {}}}"#, true)]
#[case::zero_weight(r#"{"weights": {"g": {"d": 0}}}"#, false)]
#[case::blank_identifier(r#"{"weights": {"g": {" ": 1}}}"#, false)]
#[case::list("[1]", false)]
#[case::text("\"x\"", false)]
fn router_weights_validation(#[case] settings: &str, #[case] accepted: bool) {
    let body = format!(r#"{{"router_settings": {settings}}}"#);
    let outcome = NewTeamRequest::from_value(&parse(&body));
    assert_eq!(outcome.is_ok(), accepted);
}

#[test]
fn budget_config_aliases() {
    let request = NewTeamRequest::from_value(&parse(
        r#"{"model_max_budget": {"gpt": {"budget_limit": 10, "time_period": "1d"}}}"#,
    ))
    .unwrap();
    assert_eq!(
        request.supplied().unwrap(),
        parse(r#"{"model_max_budget": {"gpt": {"max_budget": 10.0, "budget_duration": "1d"}}}"#)
    );
    let config = request
        .extra
        .model_max_budget
        .as_ref()
        .and_then(|values| values.get("gpt"))
        .unwrap();
    assert_eq!(config.max_budget, Some(Amount(Decimal::from(10_i64))));
    assert_eq!(config.tpm_limit, None);
}

#[test]
fn budget_config_alias_precedence() {
    let both = NewTeamRequest::from_value(&parse(
        r#"{"model_max_budget": {"gpt": {"max_budget": 4, "budget_limit": 3, "budget_duration": "2d", "time_period": "1d"}}}"#,
    ))
    .unwrap();
    let config = both
        .extra
        .model_max_budget
        .as_ref()
        .and_then(|values| values.get("gpt"))
        .unwrap();
    assert_eq!(config.max_budget, Some(Amount(Decimal::from(3_i64))));
    assert_eq!(config.budget_duration.as_deref(), Some("1d"));
}

#[test]
fn budget_config_empty_entry() {
    let empty = NewTeamRequest::from_value(&parse(r#"{"model_max_budget": {"gpt": {}}}"#)).unwrap();
    assert_eq!(
        empty.supplied().unwrap(),
        parse(r#"{"model_max_budget": {"gpt": {}}}"#)
    );
    let resolved = empty.resolved().unwrap();
    let entries = object(&resolved);
    let budgets = object(entries.get("model_max_budget").unwrap());
    assert_eq!(
        budgets.get("gpt").unwrap(),
        &parse(
            r#"{"max_budget": null, "budget_duration": null, "tpm_limit": null, "rpm_limit": null}"#
        )
    );
}

#[test]
fn budget_config_rejects_invalid_values() {
    NewTeamRequest::from_value(&parse(r#"{"model_max_budget": []}"#)).unwrap_err();
    NewTeamRequest::from_value(&parse(r#"{"model_max_budget": {"gpt": null}}"#)).unwrap_err();
}
