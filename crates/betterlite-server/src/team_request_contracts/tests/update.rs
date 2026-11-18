use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{object, parse};
use crate::key_fields::values::{AllowedVectorStoreIndexItem, Amount, IndexPermission};
use crate::team_request_contracts::shared::RequestProjection;
use crate::team_request_contracts::update::{PatchTeamRequest, UpdateTeamRequest};
use crate::team_request_contracts::values::PositiveInt;

const UPDATE_FIELDS: &str = r#"{
    "team_id": "team-1", "team_alias": "alias", "organization_id": "org",
    "metadata": {"k": "v"}, "tpm_limit": 10, "rpm_limit": 20, "tpd_limit": 30,
    "max_budget": 10.5, "soft_budget": 5.0, "models": ["gpt-4o"], "blocked": true,
    "budget_duration": "30d", "tags": ["t"], "model_aliases": {"m": "target"},
    "guardrails": ["g"], "policies": ["p"], "object_permission": {"models": ["gpt-4o"]},
    "disable_global_guardrails": true, "team_member_budget": 12.5,
    "team_member_budget_duration": "1mo", "team_member_rpm_limit": 13,
    "team_member_tpm_limit": 14, "team_member_key_duration": "30d",
    "allowed_passthrough_routes": ["/v1/models"], "secret_manager_settings": {"aws": {}},
    "prompts": ["pr"], "model_rpm_limit": {"gpt-4o": 5}, "model_tpm_limit": {"gpt-4o": 7},
    "default_estimated_output_tokens": 9,
    "default_estimated_output_tokens_per_model": {"gpt-4o": 11},
    "mcp_rpm_limit": {"mcp": 3},
    "allowed_vector_store_indexes": [{"index_name": "i", "index_permissions": ["read", "write"]}],
    "enforced_batch_output_expires_after": {"h": 24},
    "enforced_file_expires_after": {"h": 48},
    "router_settings": {"weights": {"g": {"d": 1}}}, "access_group_ids": ["ag"],
    "budget_limits": [{"budget_duration": "1d", "max_budget": 2}],
    "default_team_member_models": ["gpt-4o"],
    "model_max_budget": {"gpt-4o": {"max_budget": 10.0, "budget_duration": "1d"}}
}"#;

#[test]
fn team_update_all_fields() {
    let request = UpdateTeamRequest::from_value(&parse(UPDATE_FIELDS)).unwrap();
    assert_eq!(request.team_id, "team-1");
    let fields = &request.fields;
    assert_eq!(fields.team_alias.as_deref(), Some("alias"));
    assert_eq!(fields.organization_id.as_deref(), Some("org"));
    assert_eq!(
        fields.metadata.as_ref().and_then(|values| values.get("k")),
        Some(&parse(r#""v""#))
    );
    assert_eq!(fields.tpm_limit, Some(10));
    assert_eq!(fields.rpm_limit, Some(20));
    assert_eq!(fields.tpd_limit, Some(30));
    assert_eq!(fields.max_budget, Some(Amount(Decimal::new(105, 1))));
    assert_eq!(fields.soft_budget, Some(Amount(Decimal::from(5_i64))));
    assert_eq!(fields.models, Some(vec![parse(r#""gpt-4o""#)]));
    assert_eq!(fields.blocked, Some(true));
    assert_eq!(fields.budget_duration.as_deref(), Some("30d"));
    assert_eq!(fields.tags, Some(vec![parse(r#""t""#)]));
    assert_eq!(
        fields
            .model_aliases
            .as_ref()
            .and_then(|values| values.get("m")),
        Some(&parse(r#""target""#))
    );
    assert_eq!(fields.guardrails, Some(vec!["g".to_owned()]));
    assert_eq!(fields.policies, Some(vec!["p".to_owned()]));
    assert!(fields.object_permission.is_some());
    assert_eq!(fields.disable_global_guardrails, Some(true));
    assert_eq!(
        fields.team_member_budget,
        Some(Amount(Decimal::new(125, 1)))
    );
    assert_eq!(fields.team_member_budget_duration.as_deref(), Some("1mo"));
    assert_eq!(fields.team_member_rpm_limit, Some(13));
    assert_eq!(fields.team_member_tpm_limit, Some(14));
    assert_eq!(fields.team_member_key_duration.as_deref(), Some("30d"));
    assert_eq!(
        fields.allowed_passthrough_routes,
        Some(vec![parse(r#""/v1/models""#)])
    );
    assert_eq!(
        fields
            .secret_manager_settings
            .as_ref()
            .map(|values| values.len()),
        Some(1)
    );
    assert_eq!(fields.prompts, Some(vec!["pr".to_owned()]));
    assert_eq!(
        fields
            .model_rpm_limit
            .as_ref()
            .and_then(|values| values.get("gpt-4o")),
        Some(&5)
    );
    assert_eq!(
        fields
            .model_tpm_limit
            .as_ref()
            .and_then(|values| values.get("gpt-4o")),
        Some(&7)
    );
    assert_eq!(fields.estimated_output_tokens, Some(PositiveInt(9)));
    assert_eq!(
        fields
            .per_model_output_tokens
            .as_ref()
            .and_then(|values| values.get("gpt-4o")),
        Some(&PositiveInt(11))
    );
    assert_eq!(
        fields
            .mcp_rpm_limit
            .as_ref()
            .and_then(|values| values.get("mcp")),
        Some(&3)
    );
    let indexes = fields.allowed_vector_store_indexes.as_ref().unwrap();
    assert_eq!(indexes[0].index_name, "i");
    assert_eq!(
        indexes[0].index_permissions,
        vec![IndexPermission::Read, IndexPermission::Write]
    );
    assert_eq!(
        fields
            .enforced_batch_expiry
            .as_ref()
            .and_then(|values| values.get("h")),
        Some(&parse("24"))
    );
    assert_eq!(
        fields
            .enforced_file_expires_after
            .as_ref()
            .and_then(|values| values.get("h")),
        Some(&parse("48"))
    );
    assert!(fields.router_settings.is_some());
    assert_eq!(fields.access_group_ids, Some(vec!["ag".to_owned()]));
    assert_eq!(fields.budget_limits.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        fields.default_team_member_models,
        Some(vec!["gpt-4o".to_owned()])
    );
    assert_eq!(
        fields
            .model_max_budget
            .as_ref()
            .and_then(|values| values.get("gpt-4o"))
            .and_then(|config| config.max_budget),
        Some(Amount(Decimal::from(10_i64)))
    );
    assert_eq!(request.supplied().unwrap(), parse(UPDATE_FIELDS));
    let resolved = request.resolved().unwrap();
    let permission = object(object(&resolved).get("object_permission").unwrap());
    assert_eq!(permission.len(), 12);
    assert_eq!(permission.get("models"), Some(&parse(r#"["gpt-4o"]"#)));
}

#[test]
fn patch_object_permission_projection() {
    let body = r#"{"team_id": "t", "object_permission": {"models": ["gpt-4o"]}}"#;
    let request = PatchTeamRequest::from_value(&parse(body)).unwrap();
    assert_eq!(request.supplied().unwrap(), parse(body));
    let resolved = request.resolved().unwrap();
    let permission = object(object(&resolved).get("object_permission").unwrap());
    assert_eq!(permission.len(), 12);
    assert_eq!(permission.get("models"), Some(&parse(r#"["gpt-4o"]"#)));
}

#[rstest]
#[case::absent(r#"{"team_alias": "a"}"#, None)]
#[case::present(r#"{"team_id": "t", "team_alias": "a"}"#, Some("t"))]
#[case::null(r#"{"team_id": null, "team_alias": "a"}"#, None)]
#[case::blank(r#"{"team_id": "  "}"#, Some("  "))]
#[case::empty(r#"{"team_id": ""}"#, Some(""))]
fn patch_optional_team_id(#[case] body: &str, #[case] expected: Option<&str>) {
    let request = PatchTeamRequest::from_value(&parse(body)).unwrap();
    assert_eq!(request.team_id.as_deref(), expected);
    assert_eq!(request.supplied().unwrap(), parse(body));
}

#[test]
fn update_requires_team_id() {
    assert!(UpdateTeamRequest::from_value(&parse(r#"{"team_alias": "a"}"#)).is_err());
    assert!(UpdateTeamRequest::from_value(&parse(r#"{"team_id": null}"#)).is_err());
    assert!(UpdateTeamRequest::from_value(&parse(r#"{"team_id": 7}"#)).is_err());
    assert_eq!(
        UpdateTeamRequest::from_value(&parse(r#"{"team_id": "t"}"#))
            .unwrap()
            .team_id,
        "t"
    );
}

#[test]
fn update_absent_null_empty_zero() {
    let absent = UpdateTeamRequest::from_value(&parse(r#"{"team_id": "t"}"#)).unwrap();
    assert_eq!(absent.fields.max_budget, None);
    assert_eq!(absent.fields.rpm_limit, None);
    assert_eq!(absent.fields.models, None);
    assert_eq!(absent.supplied().unwrap(), parse(r#"{"team_id": "t"}"#));

    let null = UpdateTeamRequest::from_value(&parse(
        r#"{"team_id": "t", "max_budget": null, "rpm_limit": null, "models": null, "team_alias": null}"#,
    ))
    .unwrap();
    assert_eq!(null.fields.max_budget, None);
    assert_eq!(null.fields.rpm_limit, None);
    assert_eq!(
        null.supplied().unwrap(),
        parse(
            r#"{"team_id": "t", "max_budget": null, "rpm_limit": null, "models": null, "team_alias": null}"#
        )
    );

    let zero = UpdateTeamRequest::from_value(&parse(
        r#"{"team_id": "t", "max_budget": 0, "rpm_limit": 0, "models": [], "team_alias": "", "metadata": {}}"#,
    ))
    .unwrap();
    assert_eq!(zero.fields.max_budget, Some(Amount(Decimal::ZERO)));
    assert_eq!(zero.fields.rpm_limit, Some(0));
    assert_eq!(zero.fields.models, Some(Vec::new()));
    assert_eq!(zero.fields.team_alias.as_deref(), Some(""));
    assert_eq!(
        zero.supplied().unwrap(),
        parse(
            r#"{"team_id": "t", "max_budget": 0.0, "rpm_limit": 0, "models": [], "team_alias": "", "metadata": {}}"#
        )
    );
}

#[test]
fn update_budget_aliases_normalized() {
    let request = UpdateTeamRequest::from_value(&parse(
        r#"{"team_id": "t", "model_max_budget": {"gpt": {"budget_limit": 3, "time_period": "1d"}}}"#,
    ))
    .unwrap();
    assert_eq!(
        request.supplied().unwrap(),
        parse(
            r#"{"team_id": "t", "model_max_budget": {"gpt": {"max_budget": 3.0, "budget_duration": "1d"}}}"#
        )
    );
    assert!(
        UpdateTeamRequest::from_value(&parse(
            r#"{"team_id": "t", "router_settings": {"weights": {"g": {"d": 0}}}}"#,
        ))
        .is_err()
    );
    assert!(
        PatchTeamRequest::from_value(&parse(r#"{"router_settings": {"weights": []}}"#)).is_err()
    );
}
