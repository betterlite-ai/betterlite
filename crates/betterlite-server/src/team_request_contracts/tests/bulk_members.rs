use std::collections::BTreeSet;

use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{DATABASE, clocks, parse, settings};
use crate::key_fields::values::Amount;
use crate::team_request_contracts::bulk_add::{
    BulkTeamMemberAddRequest, BulkTeamMemberAddResponse, TeamMemberAddResult,
};
use crate::team_request_contracts::bulk_members::{
    BulkBudgetUpdateRequest, BulkBudgetUpdateResponse, BulkTeamMemberDeleteRequest,
    BulkTeamMemberDeleteResponse, TeamMemberBudgetPatch, TeamMemberBudgetUpdateResult,
    TeamMemberDeleteResult, TeamMemberRef,
};
use crate::team_request_contracts::create::NewTeamRequest;
use crate::team_request_contracts::metadata::TeamMetadataFieldSchema;
use crate::team_request_contracts::shared::{decode_request, to_data};
use crate::team_request_contracts::values::BudgetSource;

fn members_body(count: usize) -> String {
    let items: Vec<String> = (0..count)
        .map(|index| format!(r#"{{"user_id": "u{index}"}}"#))
        .collect();
    format!(r#"{{"members": [{}]}}"#, items.join(","))
}

#[test]
fn member_identity_missing_both_and_present_both() {
    let missing = TeamMemberRef::from_value(&parse("{}")).unwrap_err();
    assert_eq!(
        missing.to_string(),
        "Either user id or user email must be provided"
    );
    let nulls =
        TeamMemberRef::from_value(&parse(r#"{"user_id": null, "user_email": null}"#)).unwrap_err();
    assert_eq!(
        nulls.to_string(),
        "Either user id or user email must be provided"
    );
    let both =
        TeamMemberRef::from_value(&parse(r#"{"user_id": "u", "user_email": "e@x"}"#)).unwrap_err();
    assert_eq!(
        both.to_string(),
        "Each member must be identified by exactly one of user_id or user_email"
    );
    let user =
        TeamMemberRef::from_value(&parse(r#"{"user_id": "u", "user_email": null}"#)).unwrap();
    assert_eq!(user.user_id.as_deref(), Some("u"));
    assert_eq!(user.user_email, None);
    let email = TeamMemberRef::from_value(&parse(r#"{"user_email": "e@x"}"#)).unwrap();
    assert_eq!(email.user_email.as_deref(), Some("e@x"));
    assert_eq!(email.user_id, None);
    let bulk = BulkTeamMemberDeleteRequest::from_value(&parse(
        r#"{"members": [{"user_id": "u"}, {"user_id": "a", "user_email": "b"}]}"#,
    ))
    .unwrap_err();
    assert_eq!(
        bulk.to_string(),
        "Each member must be identified by exactly one of user_id or user_email"
    );
}

#[test]
fn empty_identifier_accepted() {
    let user = TeamMemberRef::from_value(&parse(r#"{"user_id": ""}"#)).unwrap();
    assert_eq!(user.user_id.as_deref(), Some(""));
    let email = TeamMemberRef::from_value(&parse(r#"{"user_email": ""}"#)).unwrap();
    assert_eq!(email.user_email.as_deref(), Some(""));
    let both =
        TeamMemberRef::from_value(&parse(r#"{"user_id": "", "user_email": ""}"#)).unwrap_err();
    assert_eq!(
        both.to_string(),
        "Each member must be identified by exactly one of user_id or user_email"
    );
    let bulk = BulkTeamMemberDeleteRequest::from_value(&parse(r#"{"members": [{"user_id": ""}]}"#))
        .unwrap();
    assert_eq!(bulk.members[0].user_id.as_deref(), Some(""));
}

#[test]
fn forbidden_vs_ignored_extra_fields() {
    TeamMemberRef::from_value(&parse(r#"{"user_id": "u", "role": "admin"}"#)).unwrap_err();
    TeamMemberBudgetPatch::from_value(
        &parse(r#"{"user_id": "u", "tpm_limit": 1, "extra": true}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    BulkTeamMemberDeleteRequest::from_value(&parse(r#"{"members": [{"user_id": "u"}], "x": 1}"#))
        .unwrap_err();
    BulkTeamMemberDeleteRequest::from_value(&parse(r#"{"members": [{"user_id": "u", "n": 1}]}"#))
        .unwrap_err();
    BulkBudgetUpdateRequest::from_value(
        &parse(r#"{"members": [{"user_id": "u"}], "x": 1}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    BulkBudgetUpdateRequest::from_value(
        &parse(r#"{"members": [{"user_id": "u", "x": 1}]}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    TeamMetadataFieldSchema::from_value(&parse(r#"{"key": "k", "x": 1}"#)).unwrap_err();
}

#[test]
fn ignored_extra_fields_survive() {
    let result: TeamMemberDeleteResult =
        decode_request(&parse(r#"{"success": true, "extra": 1}"#)).unwrap();
    assert_eq!(
        result,
        TeamMemberDeleteResult {
            user_id: None,
            user_email: None,
            success: true,
            error: None,
        }
    );
    let added =
        decode_request::<BulkTeamMemberAddRequest>(&parse(r#"{"team_id": "t", "x": 1}"#)).unwrap();
    assert_eq!(added.team_id, "t");
    NewTeamRequest::from_value(&parse(r#"{"team_id": "t", "x": 1}"#)).unwrap();
}

#[test]
fn bulk_members_one_and_five_hundred() {
    let single = BulkTeamMemberDeleteRequest::from_value(&parse(&members_body(1))).unwrap();
    assert_eq!(single.members.len(), 1);
    let full = BulkTeamMemberDeleteRequest::from_value(&parse(&members_body(500))).unwrap();
    assert_eq!(full.members.len(), 500);
    assert_eq!(full.members[499].user_id.as_deref(), Some("u499"));
    let updates = BulkBudgetUpdateRequest::from_value(
        &parse(&members_body(500)),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(updates.members.len(), 500);
    let one = BulkBudgetUpdateRequest::from_value(
        &parse(&members_body(1)),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(one.members.len(), 1);
}

#[test]
fn bulk_members_empty_and_over_limit() {
    let empty = BulkTeamMemberDeleteRequest::from_value(&parse(&members_body(0))).unwrap_err();
    assert_eq!(
        empty.to_string(),
        "Tuple should have at least 1 item after validation, not 0"
    );
    let over = BulkTeamMemberDeleteRequest::from_value(&parse(&members_body(501))).unwrap_err();
    assert_eq!(
        over.to_string(),
        "Tuple should have at most 500 items after validation, not 501"
    );
    let empty_updates = BulkBudgetUpdateRequest::from_value(
        &parse(&members_body(0)),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert_eq!(
        empty_updates.to_string(),
        "Tuple should have at least 1 item after validation, not 0"
    );
    let over_updates = BulkBudgetUpdateRequest::from_value(
        &parse(&members_body(600)),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert_eq!(
        over_updates.to_string(),
        "Tuple should have at most 500 items after validation, not 600"
    );
    BulkTeamMemberDeleteRequest::from_value(&parse("{}")).unwrap_err();
    BulkBudgetUpdateRequest::from_value(&parse("{}"), &settings(), clocks(), &DATABASE())
        .unwrap_err();
}

#[test]
fn duplicates_and_order_retained() {
    let delete = BulkTeamMemberDeleteRequest::from_value(&parse(
        r#"{"members": [{"user_id": "b"}, {"user_id": "a"}, {"user_email": "b"}, {"user_id": "b"}]}"#,
    ))
    .unwrap();
    let order: Vec<Option<&str>> = delete
        .members
        .iter()
        .map(|member| member.user_id.as_deref())
        .collect();
    assert_eq!(order, vec![Some("b"), Some("a"), None, Some("b")]);
    let updates = BulkBudgetUpdateRequest::from_value(
        &parse(
            r#"{"members": [{"user_id": "b", "max_budget_in_team": 5}, {"user_id": "a", "max_budget_in_team": 7}, {"user_id": "b", "max_budget_in_team": 9}]}"#,
        ),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    let budgets: Vec<Option<Amount>> = updates
        .members
        .iter()
        .map(|member| member.max_budget_in_team)
        .collect();
    assert_eq!(
        budgets,
        vec![
            Some(Amount(Decimal::from(5_i64))),
            Some(Amount(Decimal::from(7_i64))),
            Some(Amount(Decimal::from(9_i64)))
        ]
    );
}

#[test]
fn member_budget_patch_presence() {
    let omitted = TeamMemberBudgetPatch::from_value(
        &parse(r#"{"user_id": "u"}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(omitted.max_budget_in_team, None);
    assert_eq!(omitted.tpm_limit, None);
    assert_eq!(omitted.allowed_models, None);
    assert_eq!(
        omitted.supplied_fields,
        BTreeSet::from(["user_id".to_owned()])
    );
    assert_eq!(
        to_data(&omitted).unwrap(),
        parse(
            r#"{"user_id": "u", "user_email": null, "max_budget_in_team": null, "tpm_limit": null, "rpm_limit": null, "budget_duration": null, "allowed_models": null}"#
        )
    );

    let explicit = TeamMemberBudgetPatch::from_value(
        &parse(
            r#"{"user_email": "e@x", "max_budget_in_team": null, "tpm_limit": null, "rpm_limit": null, "budget_duration": null, "allowed_models": null}"#,
        ),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(explicit.max_budget_in_team, None);
    assert_eq!(explicit.supplied_fields.len(), 6);
    assert!(explicit.supplied_fields.contains("max_budget_in_team"));
    assert!(explicit.supplied_fields.contains("allowed_models"));
}

#[test]
fn member_budget_patch_zero_values() {
    let zero = TeamMemberBudgetPatch::from_value(
        &parse(r#"{"user_id": "u", "max_budget_in_team": 0, "tpm_limit": 0, "rpm_limit": 0, "allowed_models": []}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(zero.max_budget_in_team, Some(Amount(Decimal::ZERO)));
    assert_eq!(zero.tpm_limit, Some(0));
    assert_eq!(zero.rpm_limit, Some(0));
    assert_eq!(zero.allowed_models, Some(Vec::new()));
    assert!(zero.supplied_fields.contains("max_budget_in_team"));
    assert!(zero.supplied_fields.contains("allowed_models"));
}

#[rstest]
#[case::day("1d", true)]
#[case::month("1mo", true)]
#[case::bogus("bogus", false)]
#[case::zero("0d", false)]
#[case::empty("", false)]
fn member_budget_duration_errors(#[case] duration: &str, #[case] accepted: bool) {
    let body = format!(r#"{{"user_id": "u", "budget_duration": "{duration}"}}"#);
    let outcome =
        TeamMemberBudgetPatch::from_value(&parse(&body), &settings(), clocks(), &DATABASE());
    assert_eq!(outcome.is_ok(), accepted);
    if let Err(error) = outcome {
        assert_eq!(
            error.to_string(),
            "invalid budget_duration: use a format like '1h', '24h', '7d', or '30d'"
        );
    }
}

#[test]
fn member_budget_duration_error_ordering() {
    let missing = TeamMemberBudgetPatch::from_value(
        &parse(r#"{"budget_duration": "bogus"}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "Either user id or user email must be provided"
    );
    let duration_first = TeamMemberBudgetPatch::from_value(
        &parse(r#"{"user_id": "u", "user_email": "e@x", "budget_duration": "bogus"}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert!(
        duration_first
            .to_string()
            .starts_with("invalid budget_duration")
    );
    let both = TeamMemberBudgetPatch::from_value(
        &parse(r#"{"user_id": "u", "user_email": "e@x", "budget_duration": "1d"}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert_eq!(
        both.to_string(),
        "Each member must be identified by exactly one of user_id or user_email"
    );
}

#[test]
fn member_budget_duration_null() {
    let null_duration = TeamMemberBudgetPatch::from_value(
        &parse(r#"{"user_id": "u", "budget_duration": null}"#),
        &settings(),
        clocks(),
        &DATABASE(),
    )
    .unwrap();
    assert_eq!(null_duration.budget_duration, None);
}

#[test]
fn budget_source_enum_and_data_envelope() {
    let result: TeamMemberBudgetUpdateResult = decode_request(&parse(
        r#"{"user_id": "u", "success": true, "max_budget": 2.5, "max_budget_source": "member", "allowed_models": ["gpt-4o"]}"#,
    ))
    .unwrap();
    assert_eq!(result.max_budget, Some(Amount(Decimal::new(25, 1))));
    assert_eq!(result.max_budget_source, Some(BudgetSource::Member));
    assert_eq!(result.budget_id, None);
    assert_eq!(result.budget_duration, None);
    assert_eq!(result.allowed_models, Some(vec!["gpt-4o".to_owned()]));
    let default_source: TeamMemberBudgetUpdateResult = decode_request(&parse(
        r#"{"user_email": "e@x", "success": false, "error": "boom", "max_budget_source": "team_default"}"#,
    ))
    .unwrap();
    assert_eq!(
        default_source.max_budget_source,
        Some(BudgetSource::TeamDefault)
    );
}

#[test]
fn budget_update_result_rejects_invalid_values() {
    decode_request::<TeamMemberBudgetUpdateResult>(&parse(
        r#"{"user_id": "u", "success": true, "max_budget_source": "other"}"#,
    ))
    .unwrap_err();
    decode_request::<TeamMemberBudgetUpdateResult>(&parse(r#"{"user_id": "u"}"#)).unwrap_err();
}

#[test]
fn bulk_response_data_envelopes() {
    let result: TeamMemberBudgetUpdateResult = decode_request(&parse(
        r#"{"user_id": "u", "success": true, "max_budget": 2.5, "max_budget_source": "member", "allowed_models": ["gpt-4o"]}"#,
    ))
    .unwrap();
    let response = BulkBudgetUpdateResponse { data: vec![result] };
    assert_eq!(
        to_data(&response).unwrap(),
        parse(
            r#"{"data": [{"user_id": "u", "user_email": null, "success": true, "error": null, "budget_id": null, "max_budget": 2.5, "max_budget_source": "member", "tpm_limit": null, "rpm_limit": null, "budget_duration": null, "allowed_models": ["gpt-4o"]}]}"#
        )
    );
    let deletes = BulkTeamMemberDeleteResponse {
        data: vec![TeamMemberDeleteResult {
            user_id: Some("u".to_owned()),
            user_email: None,
            success: true,
            error: None,
        }],
    };
    assert_eq!(
        to_data(&deletes).unwrap(),
        parse(
            r#"{"data": [{"user_id": "u", "user_email": null, "success": true, "error": null}]}"#
        )
    );
}

#[test]
fn bulk_add_defaults_and_no_selection_validator() {
    let defaults =
        decode_request::<BulkTeamMemberAddRequest>(&parse(r#"{"team_id": "t"}"#)).unwrap();
    assert_eq!(defaults.members, None);
    assert_eq!(defaults.all_users, Some(false));
    assert_eq!(defaults.max_budget_in_team, None);
    let selected = decode_request::<BulkTeamMemberAddRequest>(&parse(
        r#"{"team_id": "t", "members": [{"user_id": "u", "role": "user"}], "all_users": true, "max_budget_in_team": 1.5}"#,
    ))
    .unwrap();
    assert_eq!(selected.members.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        selected.members.as_ref().unwrap()[0]
            .base
            .user_id
            .as_deref(),
        Some("u")
    );
    assert_eq!(selected.all_users, Some(true));
    assert_eq!(
        selected.max_budget_in_team,
        Some(Amount(Decimal::new(15, 1)))
    );
    let both = decode_request::<BulkTeamMemberAddRequest>(&parse(
        r#"{"team_id": "t", "members": [], "all_users": true}"#,
    ))
    .unwrap();
    assert_eq!(both.members, Some(Vec::new()));
    assert_eq!(both.all_users, Some(true));
    let null_flag = decode_request::<BulkTeamMemberAddRequest>(&parse(
        r#"{"team_id": "t", "all_users": null}"#,
    ))
    .unwrap();
    assert_eq!(null_flag.all_users, None);
    decode_request::<BulkTeamMemberAddRequest>(&parse(r#"{"members": []}"#)).unwrap_err();
}

#[test]
fn bulk_add_response_counts() {
    let response: BulkTeamMemberAddResponse = decode_request(&parse(
        r#"{"team_id": "t", "results": [{"user_id": "u", "success": true}, {"user_email": "e@x", "success": false, "error": "boom", "user_id": null, "updated_user": {"id": "u"}, "updated_team_membership": null}], "total_requested": 2, "successful_additions": 1, "failed_additions": 1, "updated_team": {"team_id": "t"}}"#,
    ))
    .unwrap();
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].updated_user, None);
    assert_eq!(response.results[0].error, None);
    assert_eq!(response.results[1].error.as_deref(), Some("boom"));
    assert_eq!(
        response.results[1]
            .updated_user
            .as_ref()
            .and_then(|values| values.get("id")),
        Some(&parse(r#""u""#))
    );
    assert_eq!(response.total_requested, 2);
    assert_eq!(response.successful_additions, 1);
    assert_eq!(response.failed_additions, 1);
    assert_eq!(
        response
            .updated_team
            .as_ref()
            .and_then(|values| values.get("team_id")),
        Some(&parse(r#""t""#))
    );
}

#[test]
fn bulk_add_response_serializes() {
    let one = BulkTeamMemberAddResponse {
        team_id: "t".to_owned(),
        results: vec![TeamMemberAddResult {
            user_id: Some("u".to_owned()),
            user_email: None,
            success: true,
            error: None,
            updated_user: None,
            updated_team_membership: None,
        }],
        total_requested: 1,
        successful_additions: 1,
        failed_additions: 0,
        updated_team: None,
    };
    assert_eq!(
        to_data(&one).unwrap(),
        parse(
            r#"{"team_id": "t", "results": [{"user_id": "u", "user_email": null, "success": true, "error": null, "updated_user": null, "updated_team_membership": null}], "total_requested": 1, "successful_additions": 1, "failed_additions": 0, "updated_team": null}"#
        )
    );
    decode_request::<BulkTeamMemberAddResponse>(&parse(
        r#"{"team_id": "t", "results": [], "total_requested": 0, "successful_additions": 0}"#,
    ))
    .unwrap_err();
    decode_request::<BulkTeamMemberAddResponse>(&parse(
        r#"{"team_id": "t", "results": [{"user_id": "u"}]}"#,
    ))
    .unwrap_err();
}
