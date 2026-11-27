#![cfg(test)]

mod support;

use rstest::rstest;

use crate::key_fields::values::{InsensitiveContains, MatchMode};
use crate::key_query_plan::sql::SqlValue;
use crate::user_contracts::search::{UserSearchTerm, UserSearchWhere};
use crate::user_search_predicate::{
    organization_membership_sql, user_search_sql, user_search_where, user_term_sql,
};

use self::support::{Fixture, database, membership_insert, organization_insert, user_insert};

fn term(user_id: Option<&str>, user_email: Option<&str>) -> UserSearchTerm {
    UserSearchTerm {
        user_id: user_id.map(|value| InsensitiveContains {
            contains: value.to_owned(),
            mode: MatchMode::Insensitive,
        }),
        user_email: user_email.map(|value| InsensitiveContains {
            contains: value.to_owned(),
            mode: MatchMode::Insensitive,
        }),
    }
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
fn predicate_no_search_gives_no_where(#[case] search: Option<&str>) {
    assert_eq!(user_search_where(search), None);
}

#[test]
fn predicate_a_single_space_is_a_search() {
    let predicate = user_search_where(Some(" ")).unwrap();
    let space = InsensitiveContains {
        contains: String::from(" "),
        mode: MatchMode::Insensitive,
    };
    let expected = UserSearchWhere {
        or: vec![
            UserSearchTerm {
                user_id: Some(space.clone()),
                user_email: None,
            },
            UserSearchTerm {
                user_id: None,
                user_email: Some(space),
            },
        ],
    };
    assert_eq!(predicate, expected);
}

#[rstest]
#[case::untrimmed("  Bob ")]
#[case::wildcards(r"a%b_c\")]
fn predicate_keeps_the_search_exactly(#[case] search: &str) {
    let predicate = user_search_where(Some(search)).unwrap();
    assert_eq!(predicate.or.len(), 2);
    assert_eq!(predicate.or[0].user_id.as_ref().unwrap().contains, search);
    assert_eq!(
        predicate.or[1].user_email.as_ref().unwrap().contains,
        search
    );
}

#[test]
fn predicate_lists_the_user_id_term_before_the_user_email_term() {
    let predicate = user_search_where(Some("bob")).unwrap();
    assert_eq!(
        predicate.or[0]
            .user_id
            .as_ref()
            .map(|filter| filter.contains.as_str()),
        Some("bob")
    );
    assert_eq!(predicate.or[0].user_email, None);
    assert_eq!(predicate.or[1].user_id, None);
    assert_eq!(
        predicate.or[1]
            .user_email
            .as_ref()
            .map(|filter| filter.contains.as_str()),
        Some("bob")
    );
}

#[test]
fn predicate_serializes_as_litellms_mapping() {
    let predicate = user_search_where(Some("Bob")).unwrap();
    assert_eq!(
        serde_json::to_value(predicate).unwrap(),
        serde_json::json!({
            "OR": [
                {"user_id": {"contains": "Bob", "mode": "insensitive"}},
                {"user_email": {"contains": "Bob", "mode": "insensitive"}}
            ]
        })
    );
}

#[test]
fn sql_term_renders_a_set_column_as_its_ilike() {
    let mut params = Vec::new();
    assert_eq!(
        user_term_sql(&term(Some("Bob"), None), &mut params),
        "\"user_id\" ILIKE $1"
    );
    assert_eq!(params, vec![SqlValue::Text(String::from("%Bob%"))]);
    let mut email_params = Vec::new();
    assert_eq!(
        user_term_sql(&term(None, Some("Bob")), &mut email_params),
        "\"user_email\" ILIKE $1"
    );
    assert_eq!(email_params, vec![SqlValue::Text(String::from("%Bob%"))]);
}

#[test]
fn sql_term_joins_both_columns_with_and() {
    let mut params = Vec::new();
    assert_eq!(
        user_term_sql(&term(Some("Bob"), Some("bob@example.com")), &mut params),
        "\"user_id\" ILIKE $1 AND \"user_email\" ILIKE $2"
    );
    assert_eq!(
        params,
        vec![
            SqlValue::Text(String::from("%Bob%")),
            SqlValue::Text(String::from("%bob@example.com%")),
        ]
    );
}

#[test]
fn sql_empty_term_renders_true() {
    let mut params = Vec::new();
    assert_eq!(user_term_sql(&term(None, None), &mut params), "TRUE");
    assert!(params.is_empty());
}

#[test]
fn sql_empty_or_renders_false() {
    let predicate = UserSearchWhere { or: Vec::new() };
    let mut params = Vec::new();
    assert_eq!(user_search_sql(&predicate, &mut params), "FALSE");
    assert!(params.is_empty());
}

#[test]
fn sql_search_ors_its_terms_inside_parentheses() {
    let predicate = user_search_where(Some("Bob")).unwrap();
    let mut params = Vec::new();
    assert_eq!(
        user_search_sql(&predicate, &mut params),
        "(\"user_id\" ILIKE $1 OR \"user_email\" ILIKE $2)"
    );
    assert_eq!(
        params,
        vec![
            SqlValue::Text(String::from("%Bob%")),
            SqlValue::Text(String::from("%Bob%")),
        ]
    );
}

#[test]
fn sql_placeholders_number_after_the_callers_params() {
    let predicate = user_search_where(Some("Bob")).unwrap();
    let mut params = vec![
        SqlValue::Text(String::from("first")),
        SqlValue::Text(String::from("second")),
    ];
    assert_eq!(
        user_search_sql(&predicate, &mut params),
        "(\"user_id\" ILIKE $3 OR \"user_email\" ILIKE $4)"
    );
    assert_eq!(
        params,
        vec![
            SqlValue::Text(String::from("first")),
            SqlValue::Text(String::from("second")),
            SqlValue::Text(String::from("%Bob%")),
            SqlValue::Text(String::from("%Bob%")),
        ]
    );
}

#[test]
fn sql_organization_membership_binds_one_list_after_the_callers_params() {
    let mut params = vec![SqlValue::Text(String::from("caller"))];
    assert_eq!(
        organization_membership_sql(&[String::from("org-1"), String::from("org-2")], &mut params),
        "\"user_id\" IN (SELECT \"user_id\" FROM \"LiteLLM_OrganizationMembership\" WHERE \"organization_id\" = ANY($2))"
    );
    assert_eq!(
        params,
        vec![
            SqlValue::Text(String::from("caller")),
            SqlValue::TextList(vec![String::from("org-1"), String::from("org-2")]),
        ]
    );
}

fn alice_seed() -> String {
    [
        user_insert("alice-1", Some("alice@example.com")),
        user_insert("alice-2", None),
        user_insert("bob-1", Some("bob@example.com")),
        user_insert("carol-9", Some("alice-carol@example.com")),
        user_insert("x alice", Some("x alice@example.com")),
    ]
    .concat()
}

async fn search_matches(
    fixture: &Fixture,
    predicate: &UserSearchWhere,
    params: &mut Vec<SqlValue>,
) -> Vec<String> {
    let fragment = user_search_sql(predicate, params);
    let sql = format!(
        "SELECT \"user_id\" FROM \"LiteLLM_UserTable\" WHERE {fragment} ORDER BY \"user_id\""
    );
    fixture.user_ids(&sql, params.as_slice()).await
}

#[tokio::test]
async fn postgres_search_matches_the_id_or_the_email_ignoring_case() {
    let fixture = database(&alice_seed()).await;
    let predicate = user_search_where(Some("ALICE")).unwrap();
    let mut params = vec![SqlValue::Text(String::from("alice-1"))];
    let fragment = user_search_sql(&predicate, &mut params);
    let sql = format!(
        "SELECT \"user_id\" FROM \"LiteLLM_UserTable\" WHERE \"user_id\" <> $1 AND {fragment} ORDER BY \"user_id\""
    );
    assert_eq!(
        fixture.user_ids(&sql, &params).await,
        vec![
            String::from("alice-2"),
            String::from("carol-9"),
            String::from("x alice"),
        ]
    );
    fixture.close().await;
}

#[tokio::test]
async fn postgres_keeps_a_leading_space_in_the_search() {
    let fixture = database(&alice_seed()).await;
    let predicate = user_search_where(Some(" alice")).unwrap();
    let mut params = Vec::new();
    assert_eq!(
        search_matches(&fixture, &predicate, &mut params).await,
        vec![String::from("x alice")]
    );
    fixture.close().await;
}

fn wildcard_seed() -> String {
    [
        user_insert("pct", Some("a%b")),
        user_insert("plain", Some("ab")),
        user_insert("slash", Some("a\\b")),
        user_insert("under", Some("a_b")),
        user_insert("x", Some("axb")),
    ]
    .concat()
}

#[tokio::test]
async fn postgres_percent_matches_any_run() {
    let fixture = database(&wildcard_seed()).await;
    let predicate = user_search_where(Some("a%b")).unwrap();
    let mut params = Vec::new();
    assert_eq!(
        search_matches(&fixture, &predicate, &mut params).await,
        vec![
            String::from("pct"),
            String::from("plain"),
            String::from("slash"),
            String::from("under"),
            String::from("x"),
        ]
    );
    fixture.close().await;
}

#[tokio::test]
async fn postgres_underscore_matches_exactly_one_character() {
    let fixture = database(&wildcard_seed()).await;
    let predicate = user_search_where(Some("a_b")).unwrap();
    let mut params = Vec::new();
    assert_eq!(
        search_matches(&fixture, &predicate, &mut params).await,
        vec![
            String::from("pct"),
            String::from("slash"),
            String::from("under"),
            String::from("x"),
        ]
    );
    fixture.close().await;
}

#[tokio::test]
async fn postgres_backslash_escapes_the_percent() {
    let fixture = database(&wildcard_seed()).await;
    let predicate = user_search_where(Some(r"a\%b")).unwrap();
    let mut params = Vec::new();
    assert_eq!(
        search_matches(&fixture, &predicate, &mut params).await,
        vec![String::from("pct")]
    );
    fixture.close().await;
}

fn membership_seed() -> String {
    [
        organization_insert("org-1"),
        organization_insert("org-2"),
        user_insert("member-a", Some("a@example.com")),
        user_insert("member-b", Some("b@example.com")),
        user_insert("outsider", Some("o@example.com")),
        membership_insert("member-a", "org-1"),
        membership_insert("member-b", "org-2"),
    ]
    .concat()
}

#[tokio::test]
async fn postgres_membership_filter_matches_the_listed_organizations() {
    let fixture = database(&membership_seed()).await;
    let mut params = Vec::new();
    let both =
        organization_membership_sql(&[String::from("org-1"), String::from("org-2")], &mut params);
    let sql =
        format!("SELECT \"user_id\" FROM \"LiteLLM_UserTable\" WHERE {both} ORDER BY \"user_id\"");
    assert_eq!(
        fixture.user_ids(&sql, &params).await,
        vec![String::from("member-a"), String::from("member-b")]
    );
    let mut single_params = Vec::new();
    let single = organization_membership_sql(&[String::from("org-2")], &mut single_params);
    let single_sql = format!(
        "SELECT \"user_id\" FROM \"LiteLLM_UserTable\" WHERE {single} ORDER BY \"user_id\""
    );
    assert_eq!(
        fixture.user_ids(&single_sql, &single_params).await,
        vec![String::from("member-b")]
    );
    fixture.close().await;
}

#[tokio::test]
async fn postgres_empty_membership_list_matches_nobody() {
    let fixture = database(&membership_seed()).await;
    let mut params = Vec::new();
    let fragment = organization_membership_sql(&[], &mut params);
    let sql = format!(
        "SELECT \"user_id\" FROM \"LiteLLM_UserTable\" WHERE {fragment} ORDER BY \"user_id\""
    );
    assert!(fixture.user_ids(&sql, &params).await.is_empty());
    fixture.close().await;
}
