use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::aliases::ModelAliases;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::auth_team_lookup::error::Detail;
use crate::identity_grants::denial::{PublicDenial, raise_public};
use crate::identity_grants::records::GrantDenial;
use crate::identity_grants::resolver::{canonical_user_id, user_models};
use crate::identity_grants::team_grants::{
    ModelAliasMap, TeamColumnsError, decode_model_aliases, team_grants, team_json_columns,
};

const FULL_TEAM: &str = r#"{
    "team_id": "team-1", "team_alias": "Team One", "tpm_limit": 1000, "rpm_limit": 100,
    "tpd_limit": 50000, "max_budget": 50.5, "soft_budget": 40.0, "spend": 12.25,
    "model_max_budget": {"gpt-4o": {"max_budget": 1.5, "budget_duration": "1d"}},
    "models": ["gpt-4o", "o3", "gpt-4o"], "blocked": true, "metadata": {"owner": "ops"},
    "litellm_model_table": {"created_by": "admin", "updated_by": "admin", "model_aliases": {"fast": "gpt-4o"}},
    "object_permission_id": "perm-1", "object_permission": {"object_permission_id": "perm-1"},
    "members_with_roles": [
        {"user_email": "user-1@example.com", "role": "user"},
        {"user_id": "user-1", "user_email": "one@example.com", "role": "admin"},
        {"user_id": "user-1", "role": "user"},
        {"user_id": "", "role": "user"}
    ]
}"#;

const MEMBERSHIP: &str = r#"{"user_id": "other", "team_id": "team-9", "spend": 3.5,
    "litellm_budget_table": {"budget_id": "budget-1", "tpm_limit": 11, "rpm_limit": 7}}"#;

fn team_row(text: &str) -> LiteLLMTeamTable {
    LiteLLMTeamTable::try_from(serde_json::from_str::<JsonData>(text).unwrap()).unwrap()
}

fn aliases(pairs: &[(&str, &str)]) -> ModelAliasMap {
    let mut map = ModelAliasMap::new();
    for &(alias, model) in pairs {
        map.insert(alias.to_owned(), model.to_owned());
    }
    map
}

#[test]
fn team_grants_project_all_eighteen_fields() {
    let team = team_row(FULL_TEAM);
    let membership: LiteLLMTeamMembership = serde_json::from_str(MEMBERSHIP).unwrap();
    let grants = team_grants(&team, Some(&membership), Some("user-1"));
    assert_eq!(grants.team_alias.as_deref(), Some("Team One"));
    let limits = (
        grants.team_tpm_limit,
        grants.team_rpm_limit,
        grants.team_tpd_limit,
    );
    assert_eq!(limits, (Some(1000_i64), Some(100_i64), Some(50000_i64)));
    let money = (
        grants.team_max_budget,
        grants.team_soft_budget,
        grants.team_spend,
    );
    let expected_money = (
        Some(Decimal::new(505, 1)),
        Some(Decimal::new(40, 0)),
        Some(Decimal::new(1225, 2)),
    );
    assert_eq!(money, expected_money);
    assert!(grants.team_model_max_budget.unwrap().contains_key("gpt-4o"));
    assert_eq!(grants.team_models, ["gpt-4o", "o3", "gpt-4o"]);
    assert!(grants.team_blocked);
    let owner = JsonData::String("ops".to_owned());
    assert_eq!(grants.team_metadata.unwrap().get("owner"), Some(&owner));
    assert_eq!(
        grants.team_model_aliases,
        Some(aliases(&[("fast", "gpt-4o")]))
    );
    let permission = grants.team_object_permission.unwrap().object_permission_id;
    let permission_ids = (
        grants.team_object_permission_id.as_deref(),
        permission.as_str(),
    );
    assert_eq!(permission_ids, (Some("perm-1"), "perm-1"));
    let member = grants.team_member.unwrap();
    let member_view = (member.base.user_email.as_deref(), member.role);
    assert_eq!(member_view, (Some("one@example.com"), MemberRole::Admin));
    let member_limits = (
        grants.team_member_spend,
        grants.team_member_tpm_limit,
        grants.team_member_rpm_limit,
    );
    let expected_member = (Some(Decimal::new(35, 1)), Some(11_i64), Some(7_i64));
    assert_eq!(member_limits, expected_member);
}

#[rstest]
#[case::exact_id(Some("user-1"), Some("one@example.com"))]
#[case::empty_id(Some(""), None)]
#[case::no_id(None, None)]
#[case::case_differs(Some("USER-1"), None)]
#[case::email_never_matches(Some("user-1@example.com"), None)]
fn team_member_first_exact_id(#[case] user_id: Option<&str>, #[case] email: Option<&str>) {
    let grants = team_grants(&team_row(FULL_TEAM), None, user_id);
    let matched = grants.team_member.map(|member| member.base);
    match user_id {
        Some("") => assert_eq!(matched.unwrap().user_id.as_deref(), Some("")),
        Some(_) | None => assert_eq!(matched.and_then(|base| base.user_email).as_deref(), email),
    }
    assert_eq!(grants.team_member_spend, None);
    assert_eq!(grants.team_member_tpm_limit, None);
    assert_eq!(grants.team_member_rpm_limit, None);
}

#[test]
fn membership_zero_negative_and_null_kept() {
    let zero: LiteLLMTeamMembership = serde_json::from_str(
        r#"{"user_id": "u", "team_id": "t", "spend": 0.0, "litellm_budget_table": {"tpm_limit": 0, "rpm_limit": -1}}"#,
    )
    .unwrap();
    let grants = team_grants(&team_row(r#"{"team_id": "t"}"#), Some(&zero), None);
    assert_eq!(grants.team_member_spend, Some(Decimal::ZERO));
    assert_eq!(grants.team_member_tpm_limit, Some(0_i64));
    assert_eq!(grants.team_member_rpm_limit, Some(-1_i64));
    let empty: LiteLLMTeamMembership =
        serde_json::from_str(r#"{"user_id": "u", "team_id": "t", "spend": null}"#).unwrap();
    let defaults = team_grants(&team_row(r#"{"team_id": "t"}"#), Some(&empty), None);
    assert_eq!(defaults.team_member_spend, None);
    assert_eq!(defaults.team_member_tpm_limit, None);
    assert_eq!(defaults.team_member_rpm_limit, None);
    assert_eq!(defaults.team_alias, None);
    assert!(defaults.team_models.is_empty());
    assert!(!defaults.team_blocked);
}

#[rstest]
#[case::object(r#"{"fast": "gpt-4o", "smart": "o3"}"#, Some(aliases(&[("fast", "gpt-4o"), ("smart", "o3")])))]
#[case::empty_map("{}", Some(ModelAliasMap::new()))]
#[case::not_json("not json", None)]
#[case::number_value(r#"{"fast": 1}"#, None)]
#[case::array("[\"fast\"]", None)]
#[case::null("null", None)]
fn alias_text_decodes_or_reads_none(#[case] text: &str, #[case] expected: Option<ModelAliasMap>) {
    let decoded = decode_model_aliases(&ModelAliases::Text(text.to_owned())).unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn alias_mapping_checked_as_given() {
    let valid: ModelAliases = serde_json::from_str(r#"{"fast": "gpt-4o"}"#).unwrap();
    let decoded = decode_model_aliases(&valid).unwrap();
    assert_eq!(decoded, Some(aliases(&[("fast", "gpt-4o")])));
    let mixed: ModelAliases = serde_json::from_str(r#"{"fast": "gpt-4o", "slow": 1}"#).unwrap();
    match decode_model_aliases(&mixed) {
        Err(TeamColumnsError::AliasNotText { alias }) => assert_eq!(alias, "slow"),
        Ok(decoded_mixed) => panic!("a numeric alias is refused: {decoded_mixed:?}"),
    }
}

#[rstest]
#[case::invalid_text(r#""{broken""#, true)]
#[case::invalid_mapping(r#"{"fast": 1}"#, false)]
fn alias_failures_drop_metadata_by_path(#[case] column: &str, #[case] keeps_metadata: bool) {
    let text = format!(
        r#"{{"team_id": "t", "metadata": {{"owner": "ops"}}, "litellm_model_table": {{"created_by": "a", "updated_by": "a", "model_aliases": {column}}}}}"#
    );
    let columns = team_json_columns(&team_row(&text));
    assert_eq!(columns.model_aliases, None);
    assert_eq!(columns.metadata.is_some(), keeps_metadata);
    let grants = team_grants(&team_row(&text), None, None);
    assert_eq!(grants.team_metadata, columns.metadata);
}

#[test]
fn no_alias_table_reads_none() {
    let columns = team_json_columns(&team_row(r#"{"team_id": "t", "metadata": {"a": 1}}"#));
    assert_eq!(columns.model_aliases, None);
    assert!(columns.metadata.unwrap().contains_key("a"));
}

#[rstest]
#[case::text(r#"["gpt-4o", "o3", "gpt-4o"]"#, &["gpt-4o", "o3", "gpt-4o"])]
#[case::one_number(r#"["gpt-4o", 1]"#, &[])]
#[case::nested(r#"[["gpt-4o"]]"#, &[])]
#[case::null("null", &[])]
fn user_models_all_text_or_empty(#[case] models: &str, #[case] expected: &[&str]) {
    let text = format!(r#"{{"user_id": "u", "models": {models}}}"#);
    let user: LiteLLMUserTable = serde_json::from_str(&text).unwrap();
    assert_eq!(user_models(&user), expected);
}

#[rstest]
#[case::row_id(Some("new-id"), Some("legacy-id"), Some("legacy-id"))]
#[case::row_without_request(None, Some("legacy-id"), Some("legacy-id"))]
#[case::empty_row_id(Some("new-id"), Some(""), Some("new-id"))]
#[case::empty_row_no_request(None, Some(""), None)]
#[case::no_row(Some("new-id"), None, Some("new-id"))]
#[case::empty_request(Some(""), None, Some(""))]
fn canonical_user_id_prefers_row_id(
    #[case] requested: Option<&str>,
    #[case] row_id: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let row = row_id.map(|user_id| {
        let text = format!(r#"{{"user_id": "{user_id}"}}"#);
        serde_json::from_str::<LiteLLMUserTable>(&text).unwrap()
    });
    assert_eq!(canonical_user_id(requested, row.as_ref()), expected);
}

#[test]
fn public_denial_envelopes() {
    let gone = raise_public(GrantDenial::UserGone {
        user_id: "user-1".to_owned(),
    });
    assert_eq!(gone.http(), None);
    let PublicDenial::UserGone(exception) = &gone else {
        panic!("a gone user raises the ProxyException: {gone:?}");
    };
    let message = "Authentication Error, user 'user-1' no longer exists.";
    assert_eq!(exception.message, message);
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("user_id"));
    assert_eq!(exception.code, "401");
    assert_eq!(gone.to_string(), message);

    let team = raise_public(GrantDenial::TeamGone {
        team_id: "team-1".to_owned(),
    });
    assert_eq!(team.http(), Some((404_u16, Detail::Error)));
    let missing = "Team doesn't exist in db. Team=team-1. Create team via `/team/new` call.";
    assert_eq!(team.to_string(), missing);

    let outsider = raise_public(GrantDenial::NotAMember {
        user_id: "user-1".to_owned(),
        team_id: "team-1".to_owned(),
    });
    assert_eq!(outsider.http(), Some((403_u16, Detail::Text)));
    let refused = "Team 'team-1' is not in your team memberships.";
    assert_eq!(outsider.to_string(), refused);
}
