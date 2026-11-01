#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;
use rust_decimal::Decimal;

use super::duplicates::duplicate_member_indexes;
use super::filters::{
    AnyFilter, FieldFilter, FilterClause, eq_filter, in_filter, team_users_filter,
};
use super::identity::MemberIdentity;
use super::matching::{addresses_member, addresses_user, same_email, with_row_email};
use crate::identity_rows::OptionalPayload;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::team_rows::member::{Member, MemberBase, MemberRole};

fn user_fixture(user_id: &str, user_email: Option<&str>) -> LiteLLMUserTable {
    LiteLLMUserTable {
        user_id: user_id.to_owned(),
        user_alias: None,
        team_id: None,
        sso_user_id: None,
        organization_id: None,
        object_permission_id: None,
        password: None,
        password_reset_required: None,
        last_breach_check_at: None,
        teams: Vec::new(),
        user_role: None,
        max_budget: None,
        spend: Decimal::ZERO,
        user_email: user_email.map(str::to_owned),
        models: Vec::new(),
        metadata: None,
        max_parallel_requests: None,
        tpm_limit: None,
        rpm_limit: None,
        budget_duration: None,
        budget_reset_at: None,
        allowed_cache_controls: Vec::new(),
        policies: Vec::new(),
        model_spend: OptionalPayload::default(),
        model_max_budget: OptionalPayload::default(),
        created_at: None,
        updated_at: None,
        organization_memberships: None,
        object_permission: None,
    }
}

fn member_fixture(user_id: Option<&str>, user_email: Option<&str>) -> Member {
    Member {
        base: MemberBase {
            user_id: user_id.map(str::to_owned),
            user_email: user_email.map(str::to_owned),
        },
        role: MemberRole::User,
    }
}

#[rstest]
#[case::identity_defaults_and_both_absent_error(None, None, false)]
#[case::empty_identifier_is_present(Some(String::new()), None, true)]
#[case::both_identifiers_allowed(Some("u1".to_owned()), Some("e1".to_owned()), true)]
fn identity_validation(
    #[case] user_id: Option<String>,
    #[case] user_email: Option<String>,
    #[case] expect_ok: bool,
) {
    let result = MemberIdentity::new(user_id, user_email);
    assert_eq!(result.is_ok(), expect_ok);
}

#[test]
fn both_absent_rejected_with_message() {
    let error = MemberIdentity::new(None, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Either user id or user email must be provided"
    );
}

#[rstest]
#[case::email_exact_case_and_whitespace(
    Some("Alice@Example.com "),
    Some("alice@example.com"),
    false
)]
#[case::null_email_never_matches(Some("a@example.com"), None, false)]
#[case::exact_email_matches(Some("a@example.com"), Some("a@example.com"), true)]
fn same_email_matching(
    #[case] candidate: Option<&str>,
    #[case] request_email: Option<&str>,
    #[case] expected: bool,
) {
    let request = MemberIdentity {
        user_id: None,
        user_email: request_email.map(str::to_owned),
    };
    assert_eq!(same_email(candidate, &request), expected);
}

#[rstest]
#[case::id_matches_same_id(Some("u1"), None, Some("u1"), None, true)]
#[case::id_matches_email_only_roster(
    Some("u1"),
    Some("a@example.com"),
    None,
    Some("a@example.com"),
    true
)]
#[case::id_rejects_other_roster_id(
    Some("u1"),
    Some("a@example.com"),
    Some("u2"),
    Some("a@example.com"),
    false
)]
fn addresses_member_matching(
    #[case] request_id: Option<&str>,
    #[case] request_email: Option<&str>,
    #[case] member_id: Option<&str>,
    #[case] member_email: Option<&str>,
    #[case] expected: bool,
) {
    let request = MemberIdentity {
        user_id: request_id.map(str::to_owned),
        user_email: request_email.map(str::to_owned),
    };
    let member = member_fixture(member_id, member_email);
    assert_eq!(addresses_member(&member, &request), expected);
}

#[test]
fn user_row_id_overrides_email() {
    let request = MemberIdentity {
        user_id: Some("u1".to_owned()),
        user_email: Some("different@example.com".to_owned()),
    };
    let user = user_fixture("u1", Some("other@example.com"));
    assert!(addresses_user(&user, &request));
}

#[rstest]
#[case::email_map_fills_only_missing(Some("u1"), None, &[("u1", "a@example.com")], Some("a@example.com"))]
#[case::email_map_miss(Some("u2"), None, &[("u1", "a@example.com")], None)]
#[case::explicit_empty_email_retained(Some("u1"), Some(""), &[("u1", "a@example.com")], Some(""))]
fn with_row_email_filling(
    #[case] user_id: Option<&str>,
    #[case] user_email: Option<&str>,
    #[case] map_entries: &[(&str, &str)],
    #[case] expected_email: Option<&str>,
) {
    let request = MemberIdentity {
        user_id: user_id.map(str::to_owned),
        user_email: user_email.map(str::to_owned),
    };
    let mut email_of = BTreeMap::new();
    for (id, email) in map_entries {
        email_of.insert((*id).to_owned(), (*email).to_owned());
    }
    let filled = with_row_email(request, &email_of);
    assert_eq!(filled.user_email, expected_email.map(str::to_owned));
}

#[rstest]
#[case::duplicate_id_or_email(
    &[(Some("u1"), None), (None, Some("e1")), (Some("u1"), None), (None, Some("e1"))],
    &[2, 3]
)]
#[case::duplicate_empty_identifier(&[(Some(""), None), (Some(""), None)], &[1])]
#[case::earlier_duplicate_still_counts(
    &[
        (Some("u1"), Some("e1")),
        (Some("u1"), Some("e2")),
        (None, Some("e2")),
    ],
    &[1, 2]
)]
#[case::duplicate_indexes_not_compacted(
    &[(Some("u1"), None), (Some("standalone"), None), (Some("u1"), None)],
    &[2]
)]
#[case::empty_input(&[], &[])]
fn duplicate_indexes(#[case] entries: &[(Option<&str>, Option<&str>)], #[case] expected: &[usize]) {
    let requests: Vec<MemberIdentity> = entries
        .iter()
        .map(|(user_id, user_email)| MemberIdentity {
            user_id: user_id.map(str::to_owned),
            user_email: user_email.map(str::to_owned),
        })
        .collect();
    let expected_set: BTreeSet<usize> = expected.iter().copied().collect();
    assert_eq!(duplicate_member_indexes(&requests), expected_set);
}

#[test]
fn in_sorted_duplicates_retained() {
    let clause = in_filter(
        "user_id",
        vec!["b".to_owned(), "a".to_owned(), "a".to_owned()],
    );
    let mut fields = BTreeMap::new();
    fields.insert(
        "user_id".to_owned(),
        FieldFilter::In {
            values: vec!["a".to_owned(), "a".to_owned(), "b".to_owned()],
        },
    );
    assert_eq!(clause, FilterClause(fields));
}

#[test]
fn equality_and_team_filters() {
    let equality = eq_filter("team_id", "t1".to_owned());
    let mut equality_fields = BTreeMap::new();
    equality_fields.insert("team_id".to_owned(), FieldFilter::Eq("t1".to_owned()));
    assert_eq!(equality, FilterClause(equality_fields));

    let team_users = team_users_filter("t1".to_owned(), vec!["u2".to_owned(), "u1".to_owned()]);
    let mut team_fields = BTreeMap::new();
    team_fields.insert("team_id".to_owned(), FieldFilter::Eq("t1".to_owned()));
    team_fields.insert(
        "user_id".to_owned(),
        FieldFilter::In {
            values: vec!["u1".to_owned(), "u2".to_owned()],
        },
    );
    assert_eq!(team_users, FilterClause(team_fields));
}

#[test]
fn or_clause_order_empty() {
    let first = eq_filter("user_id", "u1".to_owned());
    let second = eq_filter("user_id", "u2".to_owned());
    let any = AnyFilter {
        or: vec![first.clone(), second.clone()],
    };
    assert_eq!(any.or, vec![first, second]);

    let empty = AnyFilter { or: Vec::new() };
    assert!(empty.or.is_empty());
}
