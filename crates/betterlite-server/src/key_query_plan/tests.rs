#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::Expiry;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use rust_decimal::Decimal;

use super::error::KeyQueryPlanError;
use super::filters::globals::build_key_filter_conditions;
use super::filters::visibility::visibility_arms;
use super::filters::{
    KeyColumn, KeyFilters, KeyPredicate, KeyScalar, TextMatching, WhereClause, WhereEntry,
    build_key_search_where, filter_out_ui_session_tokens,
};
use super::list::{list_keys, list_keys_with};
use super::plan::KeyQueryPlan;
use super::plan::{KeyListQuery, KeyTable, build_plan};
use super::response::{KeyListEntry, KeyRow, entry_for_row};
use super::sort::{
    DEFAULT_SORT, SORT_COLUMNS, SortBy, SortColumn, SortError, SortOrder, validate_sort_column,
    validate_sort_params,
};
use super::source::KeyListSource;
use super::sql::{count_query, page_query};
use super::status::{
    KeyStatus, KeyStatusSource, build_expires_where_clause, build_status_where_clause,
    derive_key_status, not_blocked_where_clause,
};

mod postgres;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn text(value: &str) -> KeyScalar {
    KeyScalar::Text(value.to_owned())
}

fn equals(column: KeyColumn, value: &str) -> WhereClause {
    WhereClause(vec![WhereEntry::Predicate(KeyPredicate::Equals(
        column,
        text(value),
    ))])
}

fn clause_of(entries: Vec<WhereEntry>) -> WhereClause {
    WhereClause(entries)
}

fn or_of(children: Vec<WhereClause>) -> WhereEntry {
    WhereEntry::Or(children)
}

fn and_of(children: Vec<WhereClause>) -> WhereEntry {
    WhereEntry::And(children)
}

fn row(document: &str) -> KeyRow {
    let value: JsonData = serde_json::from_str(document).unwrap();
    let JsonData::Object(fields) = value else {
        panic!("a key row is a JSON object");
    };
    fields
}

fn entry_fields(entry: &KeyListEntry) -> BTreeMap<String, JsonData> {
    let document = serde_json::to_string(entry).unwrap();
    let value: JsonData = serde_json::from_str(&document).unwrap();
    let JsonData::Object(fields) = value else {
        panic!("a full key entry serializes to an object");
    };
    fields
}

fn user_table(user_id: &str, email: Option<&str>, alias: Option<&str>) -> LiteLLMUserTable {
    LiteLLMUserTable {
        user_id: user_id.to_owned(),
        user_alias: alias.map(str::to_owned),
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
        user_email: email.map(str::to_owned),
        models: Vec::new(),
        metadata: None,
        max_parallel_requests: None,
        tpm_limit: None,
        rpm_limit: None,
        budget_duration: None,
        budget_reset_at: None,
        allowed_cache_controls: Vec::new(),
        policies: Vec::new(),
        model_spend: Default::default(),
        model_max_budget: Default::default(),
        created_at: None,
        updated_at: None,
        organization_memberships: None,
        object_permission: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Page,
    Count,
    Users,
    Attach,
}

#[derive(Default)]
struct FakeSource {
    page: Vec<KeyRow>,
    total: i64,
    users: Vec<LiteLLMUserTable>,
    failure: Option<Failure>,
    calls: Mutex<Vec<String>>,
    attach_calls: Mutex<usize>,
}

impl FakeSource {
    fn record(&self, name: &str) {
        self.calls.lock().unwrap().push(name.to_owned());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn attach_calls(&self) -> usize {
        *self.attach_calls.lock().unwrap()
    }

    fn failed(&self, stage: Failure) -> bool {
        self.failure == Some(stage)
    }
}

impl KeyListSource for FakeSource {
    fn fetch_page(
        &self,
        _plan: &KeyQueryPlan,
    ) -> impl std::future::Future<Output = Result<Vec<KeyRow>, KeyQueryPlanError>> + Send {
        self.record("page");
        let outcome = if self.failed(Failure::Page) {
            Err(KeyQueryPlanError::MissingToken)
        } else {
            Ok(self.page.clone())
        };
        async move { outcome }
    }

    fn count(
        &self,
        _plan: &KeyQueryPlan,
    ) -> impl std::future::Future<Output = Result<i64, KeyQueryPlanError>> + Send {
        self.record("count");
        let outcome = if self.failed(Failure::Count) {
            Err(KeyQueryPlanError::MissingToken)
        } else {
            Ok(self.total)
        };
        async move { outcome }
    }

    fn find_users(
        &self,
        _ids: Vec<String>,
    ) -> impl std::future::Future<Output = Result<Vec<LiteLLMUserTable>, KeyQueryPlanError>> + Send
    {
        self.record("users");
        let outcome = if self.failed(Failure::Users) {
            Err(KeyQueryPlanError::MissingToken)
        } else {
            Ok(self.users.clone())
        };
        async move { outcome }
    }

    fn attach_object_permission(
        &self,
        _row: &mut KeyRow,
    ) -> impl std::future::Future<Output = Result<(), KeyQueryPlanError>> + Send {
        self.record("attach");
        *self.attach_calls.lock().unwrap() += 1;
        let outcome = if self.failed(Failure::Attach) {
            Err(KeyQueryPlanError::MissingToken)
        } else {
            Ok(())
        };
        async move { outcome }
    }
}

#[test]
fn ui_team_null_or_not_dashboard() {
    let expected = clause_of(vec![or_of(vec![
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
            KeyColumn::TeamId,
        ))]),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::NotEquals(
            KeyColumn::TeamId,
            text("litellm-dashboard"),
        ))]),
    ])]);
    assert_eq!(filter_out_ui_session_tokens(), expected);
}

#[test]
fn own_admin_member_visibility() {
    let filters = KeyFilters {
        user_id: Some(String::from("alice")),
        admin_team_ids: Some(vec![String::from("team-a")]),
        member_team_ids: Some(vec![String::from("team-a"), String::from("team-b")]),
        ..KeyFilters::default()
    };
    let own = equals(KeyColumn::UserId, "alice");
    let admin = WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
        KeyColumn::TeamId,
        vec![String::from("team-a")],
    ))]);
    let member = WhereClause(vec![and_of(vec![
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
            KeyColumn::TeamId,
            vec![String::from("team-b")],
        ))]),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
            KeyColumn::UserId,
        ))]),
    ])]);
    let expected = clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        clause_of(vec![or_of(vec![own, admin, member])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&filters, now()), expected);
}

#[test]
fn own_arm_org_and_excluded_team_not_global() {
    let filters = KeyFilters {
        user_id: Some(String::from("alice")),
        exclude_team_id: Some(String::from("team-x")),
        organization_id: Some(String::from("org-1")),
        admin_team_ids: Some(vec![String::from("team-a")]),
        ..KeyFilters::default()
    };
    let own = clause_of(vec![
        WhereEntry::Predicate(KeyPredicate::Equals(KeyColumn::UserId, text("alice"))),
        WhereEntry::Predicate(KeyPredicate::NotEquals(KeyColumn::TeamId, text("team-x"))),
        WhereEntry::Predicate(KeyPredicate::Equals(
            KeyColumn::OrganizationId,
            text("org-1"),
        )),
    ]);
    let admin = WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
        KeyColumn::TeamId,
        vec![String::from("team-a")],
    ))]);
    let expected = clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        clause_of(vec![or_of(vec![own, admin])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&filters, now()), expected);
    // organization_id and exclude_team_id never become global filters of their own
    let without_teams = KeyFilters {
        user_id: Some(String::from("alice")),
        exclude_team_id: Some(String::from("team-x")),
        organization_id: Some(String::from("org-1")),
        ..KeyFilters::default()
    };
    assert_eq!(
        visibility_arms(&without_teams).len(),
        1,
        "one own-key arm carries both filters"
    );
}

#[test]
fn creator_none_empty_current_teams() {
    let base = KeyFilters {
        user_id: Some(String::from("alice")),
        include_created_by_keys: true,
        ..KeyFilters::default()
    };
    let own = equals(KeyColumn::UserId, "alice");
    let created = equals(KeyColumn::CreatedBy, "alice");
    let ui = filter_out_ui_session_tokens();
    // two arms OR together under the UI-session exclusion
    let expected = clause_of(vec![and_of(vec![
        ui.clone(),
        clause_of(vec![or_of(vec![own.clone(), created.clone()])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&base, now()), expected);

    let no_teams = KeyFilters {
        member_team_ids: Some(Vec::new()),
        ..base.clone()
    };
    let empty_created = clause_of(vec![and_of(vec![
        created.clone(),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
            KeyColumn::TeamId,
        ))]),
    ])]);
    let expected = clause_of(vec![and_of(vec![
        ui.clone(),
        clause_of(vec![or_of(vec![own.clone(), empty_created])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&no_teams, now()), expected);

    let current = KeyFilters {
        member_team_ids: Some(vec![String::from("team-a"), String::from("team-b")]),
        ..base
    };
    let scoped_teams = clause_of(vec![or_of(vec![
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
            KeyColumn::TeamId,
            vec![String::from("team-a"), String::from("team-b")],
        ))]),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
            KeyColumn::TeamId,
        ))]),
    ])]);
    let scoped_created = clause_of(vec![and_of(vec![created, scoped_teams])]);
    let expected = clause_of(vec![and_of(vec![
        ui,
        clause_of(vec![or_of(vec![own, scoped_created])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&current, now()), expected);
}

#[test]
fn member_service_accounts_only() {
    let member_only = KeyFilters {
        member_team_ids: Some(vec![String::from("team-a")]),
        ..KeyFilters::default()
    };
    let mut entries = filter_out_ui_session_tokens().0;
    entries.push(and_of(vec![
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
            KeyColumn::TeamId,
            vec![String::from("team-a")],
        ))]),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
            KeyColumn::UserId,
        ))]),
    ]));
    assert_eq!(
        build_key_filter_conditions(&member_only, now()),
        clause_of(entries)
    );

    let overlapping = KeyFilters {
        admin_team_ids: Some(vec![String::from("team-a"), String::from("team-b")]),
        member_team_ids: Some(vec![String::from("team-a"), String::from("team-b")]),
        ..KeyFilters::default()
    };
    // every member team is already an admin team, so no service-account arm is added
    let arms = visibility_arms(&overlapping);
    assert_eq!(arms.len(), 1);
    assert_eq!(
        arms[0],
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
            KeyColumn::TeamId,
            vec![String::from("team-a"), String::from("team-b")],
        ))])
    );
}

#[test]
fn no_visibility_arm() {
    let filters = KeyFilters::default();
    assert!(visibility_arms(&filters).is_empty());
    assert_eq!(
        build_key_filter_conditions(&filters, now()),
        filter_out_ui_session_tokens()
    );
}

#[test]
fn all_global_filters_intersect() {
    let filters = KeyFilters {
        key_alias: Some(String::from("alias")),
        key_hash: Some(String::from("hash")),
        search: Some(String::from("term")),
        team_id: Some(String::from("team")),
        project_id: Some(String::from("project")),
        access_group_id: Some(String::from("group")),
        agent_id: Some(String::from("agent")),
        expires_filter: Some(String::from("active")),
        status_filter: Some(String::from("active")),
        ..KeyFilters::default()
    };
    let expected = clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        equals(KeyColumn::KeyAlias, "alias"),
        equals(KeyColumn::Token, "hash"),
        build_key_search_where("term"),
        equals(KeyColumn::TeamId, "team"),
        equals(KeyColumn::ProjectId, "project"),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::HasSome(
            KeyColumn::AccessGroupIds,
            vec![String::from("group")],
        ))]),
        equals(KeyColumn::AgentId, "agent"),
        build_expires_where_clause("active", now()),
        build_status_where_clause(Some("active"), now()).unwrap(),
    ])]);
    assert_eq!(build_key_filter_conditions(&filters, now()), expected);
}

#[test]
fn exact_token_or_alias_substring_search() {
    let expected = clause_of(vec![or_of(vec![
        equals(KeyColumn::Token, "abc"),
        WhereClause(vec![WhereEntry::Predicate(
            KeyPredicate::ContainsInsensitive(KeyColumn::KeyAlias, String::from("abc")),
        )]),
    ])]);
    assert_eq!(build_key_search_where("abc"), expected);

    let substring = KeyFilters {
        user_id: Some(String::from("alice")),
        key_alias: Some(String::from("Ali")),
        user_matching: TextMatching::Substring,
        key_alias_matching: TextMatching::Substring,
        ..KeyFilters::default()
    };
    let mut own_entries = filter_out_ui_session_tokens().0;
    own_entries.push(WhereEntry::Predicate(KeyPredicate::ContainsInsensitive(
        KeyColumn::UserId,
        String::from("alice"),
    )));
    let expected_substring = clause_of(vec![and_of(vec![
        clause_of(own_entries),
        WhereClause(vec![WhereEntry::Predicate(
            KeyPredicate::ContainsInsensitive(KeyColumn::KeyAlias, String::from("Ali")),
        )]),
    ])]);
    assert_eq!(
        build_key_filter_conditions(&substring, now()),
        expected_substring
    );

    let exact = KeyFilters {
        user_id: Some(String::from("alice")),
        key_alias: Some(String::from("Ali")),
        ..KeyFilters::default()
    };
    let mut exact_entries = filter_out_ui_session_tokens().0;
    exact_entries.push(WhereEntry::Predicate(KeyPredicate::Equals(
        KeyColumn::UserId,
        text("alice"),
    )));
    let expected_exact = clause_of(vec![and_of(vec![
        clause_of(exact_entries),
        equals(KeyColumn::KeyAlias, "Ali"),
    ])]);
    assert_eq!(build_key_filter_conditions(&exact, now()), expected_exact);
}

#[test]
fn empty_filters_skip() {
    let filters = KeyFilters {
        user_id: Some(String::new()),
        team_id: Some(String::new()),
        organization_id: Some(String::new()),
        key_alias: Some(String::new()),
        key_hash: Some(String::new()),
        exclude_team_id: Some(String::new()),
        admin_team_ids: Some(Vec::new()),
        member_team_ids: Some(Vec::new()),
        include_created_by_keys: true,
        project_id: Some(String::new()),
        access_group_id: Some(String::new()),
        agent_id: Some(String::new()),
        expires_filter: Some(String::new()),
        search: Some(String::new()),
        status_filter: Some(String::new()),
        ..KeyFilters::default()
    };
    assert!(visibility_arms(&filters).is_empty());
    assert_eq!(
        build_key_filter_conditions(&filters, now()),
        filter_out_ui_session_tokens()
    );
}

#[test]
fn expiry_before_equal_after() {
    let before = KeyStatusSource {
        blocked: None,
        expires: Some(Expiry::Text(String::from("2025-05-31T23:59:59Z"))),
    };
    let equal = KeyStatusSource {
        blocked: None,
        expires: Some(Expiry::Text(String::from("2025-06-01T00:00:00Z"))),
    };
    let after = KeyStatusSource {
        blocked: None,
        expires: Some(Expiry::Text(String::from("2025-06-01T00:00:01Z"))),
    };
    assert_eq!(
        derive_key_status(&before, now()).unwrap(),
        KeyStatus::Expired
    );
    assert_eq!(derive_key_status(&equal, now()).unwrap(), KeyStatus::Active);
    assert_eq!(derive_key_status(&after, now()).unwrap(), KeyStatus::Active);

    let expired_clause = build_expires_where_clause("expired", now());
    assert_eq!(
        expired_clause,
        clause_of(vec![and_of(vec![
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNotNull(
                KeyColumn::Expires
            ))]),
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::Before(
                KeyColumn::Expires,
                now()
            ))]),
        ])])
    );
    let active_clause = build_expires_where_clause("active", now());
    assert_eq!(
        active_clause,
        clause_of(vec![or_of(vec![
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
                KeyColumn::Expires
            ))]),
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::AtOrAfter(
                KeyColumn::Expires,
                now()
            ))]),
        ])])
    );
}

#[test]
fn blocked_null_false_true() {
    let unblocked = KeyStatusSource {
        blocked: Some(false),
        expires: None,
    };
    let missing = KeyStatusSource {
        blocked: None,
        expires: None,
    };
    let blocked = KeyStatusSource {
        blocked: Some(true),
        expires: None,
    };
    assert_eq!(
        derive_key_status(&missing, now()).unwrap(),
        KeyStatus::Active
    );
    assert_eq!(
        derive_key_status(&unblocked, now()).unwrap(),
        KeyStatus::Active
    );
    assert_eq!(
        derive_key_status(&blocked, now()).unwrap(),
        KeyStatus::Revoked
    );

    assert_eq!(
        not_blocked_where_clause(),
        clause_of(vec![or_of(vec![
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
                KeyColumn::Blocked
            ))]),
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::Equals(
                KeyColumn::Blocked,
                KeyScalar::Bool(false),
            ))]),
        ])])
    );
    assert_eq!(
        build_status_where_clause(Some("revoked"), now()),
        Some(WhereClause(vec![WhereEntry::Predicate(
            KeyPredicate::Equals(KeyColumn::Blocked, KeyScalar::Bool(true),)
        )]))
    );
}

#[test]
fn revoked_precedes_expiry() {
    let revoked = KeyStatusSource {
        blocked: Some(true),
        expires: Some(Expiry::Text(String::from("2020-01-01T00:00:00Z"))),
    };
    assert_eq!(
        derive_key_status(&revoked, now()).unwrap(),
        KeyStatus::Revoked
    );
}

#[test]
fn naive_expiry_utc() {
    let naive = KeyStatusSource {
        blocked: None,
        expires: Some(Expiry::Text(String::from("2025-05-31T12:00:00"))),
    };
    assert_eq!(
        derive_key_status(&naive, now()).unwrap(),
        KeyStatus::Expired
    );
    let naive_future = KeyStatusSource {
        blocked: None,
        expires: Some(Expiry::Text(String::from("2025-06-01T12:00:00"))),
    };
    assert_eq!(
        derive_key_status(&naive_future, now()).unwrap(),
        KeyStatus::Active
    );
    let offset = KeyStatusSource {
        blocked: None,
        expires: Some(Expiry::Text(String::from("2025-06-01T01:00:00+02:00"))),
    };
    assert_eq!(
        derive_key_status(&offset, now()).unwrap(),
        KeyStatus::Expired,
        "01:00+02:00 is 23:00Z on the day before"
    );
}

#[test]
fn unknown_expiry_status_ignored_here() {
    let filters = KeyFilters {
        expires_filter: Some(String::from("EXPIRED")),
        status_filter: Some(String::from("bogus")),
        ..KeyFilters::default()
    };
    assert_eq!(
        build_key_filter_conditions(&filters, now()),
        filter_out_ui_session_tokens()
    );
    let deleted = KeyFilters {
        status_filter: Some(String::from("deleted")),
        ..KeyFilters::default()
    };
    assert_eq!(
        build_key_filter_conditions(&deleted, now()),
        filter_out_ui_session_tokens(),
        "deleted adds no status predicate"
    );
    assert_eq!(build_status_where_clause(Some("deleted"), now()), None);
    assert_eq!(build_status_where_clause(None, now()), None);
}

#[test]
fn sort_six_columns_casefolded_order() {
    let wires: Vec<&str> = SORT_COLUMNS.iter().map(|column| column.wire()).collect();
    assert_eq!(
        wires,
        vec![
            "spend",
            "max_budget",
            "created_at",
            "updated_at",
            "token",
            "key_alias"
        ]
    );
    for column in SORT_COLUMNS {
        assert_eq!(SortColumn::from_wire(column.wire()), Some(column));
        assert_eq!(
            validate_sort_column(column.wire(), "DESC"),
            Ok(SortBy {
                column,
                order: SortOrder::Desc,
            })
        );
        assert_eq!(
            validate_sort_column(column.wire(), "Asc").unwrap().order,
            SortOrder::Asc
        );
        assert_eq!(
            validate_sort_column(column.wire(), "aSc").unwrap().order,
            SortOrder::Asc
        );
    }
    assert_eq!(SortColumn::from_wire("user_id"), None);
}

#[test]
fn sort_missing_skips_invalid_order() {
    assert_eq!(validate_sort_params(None, "sideways"), Ok(None));
    assert_eq!(
        validate_sort_params(Some("spend"), "sideways"),
        Err(SortError::Order)
    );
}

#[test]
fn sort_errors_400() {
    let error = validate_sort_params(Some("user_id"), "asc").unwrap_err();
    assert_eq!(
        error,
        SortError::Column {
            columns: String::from("spend, max_budget, created_at, updated_at, token, key_alias"),
        }
    );
    assert_eq!(
        error.to_string(),
        "Invalid sort column. Must be one of: spend, max_budget, created_at, updated_at, token, key_alias"
    );
    assert_eq!(SortError::STATUS_CODE, 400);
    let order = validate_sort_params(Some("token"), "up").unwrap_err();
    assert_eq!(order, SortError::Order);
    assert_eq!(
        order.to_string(),
        "Invalid sort order. Must be 'asc' or 'desc'"
    );
    assert_eq!(SortError::STATUS_CODE, 400);
}

#[test]
fn default_tiebreaker_only() {
    let query = KeyListQuery::new(now(), 3, 4);
    let plan = build_plan(&query).unwrap();
    assert_eq!(plan.sort, DEFAULT_SORT.to_vec());
    assert_eq!(plan.skip, 8);
    assert_eq!(plan.take, 4);

    let explicit = KeyListQuery {
        sort_by: Some(String::from("spend")),
        sort_order: String::from("ASC"),
        ..KeyListQuery::new(now(), 1, 10)
    };
    let plan = build_plan(&explicit).unwrap();
    assert_eq!(
        plan.sort,
        vec![SortBy {
            column: SortColumn::Spend,
            order: SortOrder::Asc,
        }],
        "an explicit sort keeps no token tie-breaker"
    );
}

#[test]
fn deleted_table_no_live_relations() {
    let query = KeyListQuery {
        filters: KeyFilters {
            status_filter: Some(String::from("deleted")),
            ..KeyFilters::default()
        },
        ..KeyListQuery::new(now(), 1, 10)
    };
    let plan = build_plan(&query).unwrap();
    assert_eq!(plan.table, KeyTable::Deleted);
    let page = page_query(&plan);
    assert!(page.sql.contains("\"LiteLLM_DeletedVerificationToken\""));
    assert!(!page.sql.contains("LEFT JOIN"));
    assert!(!page.sql.contains("LiteLLM_BudgetTable"));
    let count = count_query(&plan);
    assert!(count.sql.contains("\"LiteLLM_DeletedVerificationToken\""));
    assert!(!count.sql.contains("LiteLLM_BudgetTable"));
}

#[tokio::test]
async fn empty_page_zero_pages() {
    let source = FakeSource::default();
    let mut query = KeyListQuery::new(now(), 1, 10);
    query.return_full_object = true;
    let response = list_keys_with(&source, &query).await.unwrap();
    assert!(response.keys.is_empty());
    assert_eq!(response.total_count, Some(0));
    assert_eq!(response.current_page, Some(1));
    assert_eq!(response.total_pages, Some(0));
    assert_eq!(source.attach_calls(), 0);
}

#[test]
fn dump_fallback() {
    // the mapping a pydantic v1 dict() dump would give: organization_id and no model attributes
    let mapping = row(
        r#"{"token":"hash-1","key_alias":"a","organization_id":"org-1","litellm_budget_table":{"budget_id":"b1"}}"#,
    );
    let entry = entry_for_row(mapping, KeyTable::Live, true).unwrap();
    let fields = entry_fields(&entry);
    assert_eq!(
        fields.get("org_id"),
        Some(&JsonData::String(String::from("org-1")))
    );
    assert!(!fields.contains_key("organization_id"));
    assert!(matches!(
        fields.get("litellm_budget_table"),
        Some(JsonData::Object(_))
    ));
}

#[test]
fn full_live_vs_archived_rows() {
    let live = row(r#"{"token":"live-1","key_alias":"a"}"#);
    let entry = entry_for_row(live, KeyTable::Live, true).unwrap();
    assert!(matches!(entry, KeyListEntry::Auth(_)));

    let archived = row(
        r#"{"id":"d1","token":"del-1","deleted_at":"2025-01-01T00:00:00","deleted_by":"admin","deleted_by_api_key":"op-hash","litellm_changed_by":null}"#,
    );
    let entry = entry_for_row(archived.clone(), KeyTable::Deleted, true).unwrap();
    let fields = entry_fields(&entry);
    assert_eq!(
        fields.get("deleted_by"),
        Some(&JsonData::String(String::from("admin")))
    );
    assert_eq!(
        fields.get("deleted_by_api_key"),
        Some(&JsonData::String(String::from("op-hash")))
    );
    assert_eq!(
        fields.get("token"),
        Some(&JsonData::String(String::from("del-1")))
    );
    assert!(matches!(
        entry_for_row(archived, KeyTable::Deleted, false).unwrap(),
        KeyListEntry::Token(token) if token == "del-1"
    ));
}

#[derive(Clone)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn page_then_count_not_snapshot() {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let source = FakeSource {
        page: vec![row(r#"{"token":"k1"}"#)],
        total: 7,
        ..FakeSource::default()
    };
    let query = KeyListQuery::new(now(), 2, 2);
    let response = list_keys_with(&source, &query).await.unwrap();
    assert_eq!(
        source.calls(),
        vec![
            String::from("page"),
            String::from("count"),
            String::from("attach")
        ]
    );
    // the page holds one row while the count already saw seven, so no snapshot spans both reads
    assert_eq!(response.keys.len(), 1);
    assert_eq!(response.total_count, Some(7));
    assert_eq!(response.total_pages, Some(4));
    assert_eq!(response.current_page, Some(2));
    drop(guard);
    let bytes = captured.0.lock().unwrap().clone();
    let output = String::from_utf8(bytes).unwrap();
    assert!(output.contains("Filter conditions"));
    assert!(output.contains("skip=2"));
    assert!(output.contains("take=2"));
    assert!(output.contains("Fetched keys"));
    assert!(output.contains("Total count of keys"));
}

#[tokio::test]
async fn expand_owner_and_creator() {
    let source = FakeSource {
        page: vec![row(
            r#"{"token":"k1","user_id":"alice","created_by":"bob"}"#,
        )],
        total: 1,
        users: vec![
            user_table("alice", Some("alice@example.com"), Some("Alice")),
            user_table("bob", Some("bob@example.com"), None),
        ],
        ..FakeSource::default()
    };
    let mut query = KeyListQuery::new(now(), 1, 10);
    query.expand = vec![String::from("user")];
    let response = list_keys_with(&source, &query).await.unwrap();
    let fields = entry_fields(&response.keys[0]);
    let Some(JsonData::Object(user)) = fields.get("user") else {
        panic!("the owner expansion is an object");
    };
    assert_eq!(
        user.get("user_id"),
        Some(&JsonData::String(String::from("alice")))
    );
    let Some(JsonData::Object(creator)) = fields.get("created_by_user") else {
        panic!("the creator expansion is an object");
    };
    assert_eq!(
        creator.get("user_email"),
        Some(&JsonData::String(String::from("bob@example.com")))
    );
    assert_eq!(creator.get("user_alias"), Some(&JsonData::Null));
    // expand=user alone returns full objects
    assert!(matches!(response.keys[0], KeyListEntry::Auth(_)));
}

#[tokio::test]
async fn expand_missing_users_and_unknown_names() {
    let source = FakeSource {
        page: vec![row(
            r#"{"token":"k1","user_id":"ghost","created_by":"phantom"}"#,
        )],
        total: 1,
        ..FakeSource::default()
    };
    let mut query = KeyListQuery::new(now(), 1, 10);
    query.expand = vec![String::from("user")];
    let response = list_keys_with(&source, &query).await.unwrap();
    let fields = entry_fields(&response.keys[0]);
    // the expansions were not found, so the shared auth shape carries them as null
    assert_eq!(fields.get("user"), Some(&JsonData::Null));
    assert_eq!(fields.get("created_by_user"), Some(&JsonData::Null));
    assert!(source.calls().contains(&String::from("users")));

    let unknown = FakeSource {
        page: vec![row(r#"{"token":"k1","user_id":"alice"}"#)],
        total: 1,
        ..FakeSource::default()
    };
    let mut query = KeyListQuery::new(now(), 1, 10);
    query.expand = vec![String::from("bogus")];
    let response = list_keys_with(&unknown, &query).await.unwrap();
    assert!(!unknown.calls().contains(&String::from("users")));
    assert!(matches!(response.keys[0], KeyListEntry::Token(_)));
}

#[tokio::test]
async fn token_only_still_attaches_permission() {
    let source = FakeSource {
        page: vec![row(r#"{"token":"k1"}"#), row(r#"{"token":"k2"}"#)],
        total: 2,
        ..FakeSource::default()
    };
    let query = KeyListQuery::new(now(), 1, 10);
    let response = list_keys_with(&source, &query).await.unwrap();
    assert_eq!(source.attach_calls(), 2);
    assert!(
        response
            .keys
            .iter()
            .all(|entry| matches!(entry, KeyListEntry::Token(_)))
    );

    let failing = FakeSource {
        page: vec![row(r#"{"token":"k1"}"#)],
        total: 1,
        failure: Some(Failure::Attach),
        ..FakeSource::default()
    };
    let error = list_keys_with(&failing, &query).await.unwrap_err();
    assert!(matches!(error, KeyQueryPlanError::MissingToken));
}

#[tokio::test]
async fn query_failures_propagate() {
    let invalid_sort = KeyListQuery {
        sort_by: Some(String::from("created")),
        ..KeyListQuery::new(now(), 1, 10)
    };
    let untouched = FakeSource::default();
    let error = list_keys_with(&untouched, &invalid_sort).await.unwrap_err();
    assert!(matches!(
        error,
        KeyQueryPlanError::Sort(SortError::Column { .. })
    ));
    assert!(
        untouched.calls().is_empty(),
        "no query runs without a valid sort"
    );

    let query = KeyListQuery::new(now(), 1, 10);
    let page_failure = FakeSource {
        failure: Some(Failure::Page),
        ..FakeSource::default()
    };
    let error = list_keys_with(&page_failure, &query).await.unwrap_err();
    assert!(matches!(error, KeyQueryPlanError::MissingToken));
    assert_eq!(page_failure.calls(), vec![String::from("page")]);

    let count_failure = FakeSource {
        total: 3,
        failure: Some(Failure::Count),
        ..FakeSource::default()
    };
    let error = list_keys_with(&count_failure, &query).await.unwrap_err();
    assert!(matches!(error, KeyQueryPlanError::MissingToken));
    assert_eq!(
        count_failure.calls(),
        vec![String::from("page"), String::from("count")]
    );

    let users_failure = FakeSource {
        page: vec![row(r#"{"token":"k1","user_id":"alice"}"#)],
        total: 1,
        failure: Some(Failure::Users),
        ..FakeSource::default()
    };
    let mut expanded = KeyListQuery::new(now(), 1, 10);
    expanded.expand = vec![String::from("user")];
    let error = list_keys_with(&users_failure, &expanded).await.unwrap_err();
    assert!(matches!(error, KeyQueryPlanError::MissingToken));
    assert!(users_failure.calls().contains(&String::from("users")));

    let error = list_keys(None, &query).await.unwrap_err();
    assert!(matches!(
        error,
        KeyQueryPlanError::Database(DbError::NoDbConnected)
    ));
}

#[test]
fn pagination_overflow_is_typed() {
    let query = KeyListQuery::new(now(), i64::MAX, 4);
    assert!(matches!(
        build_plan(&query),
        Err(KeyQueryPlanError::Pagination)
    ));
    let zero = KeyListQuery::new(now(), 1, 0);
    assert!(matches!(
        build_plan(&zero),
        Err(KeyQueryPlanError::Pagination)
    ));
}
