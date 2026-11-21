use betterlite_db::identity_rows::key::Expiry;

use crate::key_query_plan::filters::globals::build_key_filter_conditions;
use crate::key_query_plan::filters::visibility::visibility_arms;
use crate::key_query_plan::filters::{
    KeyColumn, KeyFilters, KeyScalar, TextMatching, Where, WhereClause, build_key_search_where,
    filter_out_ui_session_tokens,
};
use crate::key_query_plan::status::{
    KeyStatus, KeyStatusSource, build_expires_where_clause, build_status_where_clause,
    derive_key_status, not_blocked_where_clause,
};

use super::{and_of, clause_of, equals, key_filters, now, or_of, text};

#[test]
fn ui_team_null_or_not_dashboard() {
    let expected = clause_of(vec![or_of(vec![
        vec![Where::IsNull(KeyColumn::TeamId)],
        vec![Where::NotEquals(
            KeyColumn::TeamId,
            text("litellm-dashboard"),
        )],
    ])]);
    assert_eq!(filter_out_ui_session_tokens(), expected);
}

#[test]
fn own_admin_member_visibility() {
    let mut filters = key_filters();
    filters.user_id = Some(String::from("alice"));
    filters.admin_team_ids = Some(vec![String::from("team-a")]);
    filters.member_team_ids = Some(vec![String::from("team-a"), String::from("team-b")]);
    let own = equals(KeyColumn::UserId, "alice");
    let admin = vec![Where::In(KeyColumn::TeamId, vec![String::from("team-a")])];
    let member = vec![and_of(vec![
        vec![Where::In(KeyColumn::TeamId, vec![String::from("team-b")])],
        vec![Where::IsNull(KeyColumn::UserId)],
    ])];
    let expected = clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        clause_of(vec![or_of(vec![own, admin, member])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&filters, now()), expected);
}

#[test]
fn own_arm_org_and_excluded_team_not_global() {
    let mut filters = key_filters();
    filters.user_id = Some(String::from("alice"));
    filters.exclude_team_id = Some(String::from("team-x"));
    filters.organization_id = Some(String::from("org-1"));
    filters.admin_team_ids = Some(vec![String::from("team-a")]);
    let own = clause_of(vec![
        Where::Equals(KeyColumn::UserId, text("alice")),
        Where::NotEquals(KeyColumn::TeamId, text("team-x")),
        Where::Equals(KeyColumn::OrganizationId, text("org-1")),
    ]);
    let admin = vec![Where::In(KeyColumn::TeamId, vec![String::from("team-a")])];
    let expected = clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        clause_of(vec![or_of(vec![own, admin])]),
    ])]);
    assert_eq!(build_key_filter_conditions(&filters, now()), expected);
    // organization_id and exclude_team_id never become global filters of their own
    let mut without_teams = key_filters();
    without_teams.user_id = Some(String::from("alice"));
    without_teams.exclude_team_id = Some(String::from("team-x"));
    without_teams.organization_id = Some(String::from("org-1"));
    assert_eq!(
        visibility_arms(&without_teams).len(),
        1,
        "one own-key arm carries both filters"
    );
}

#[test]
fn creator_none_empty_current_teams() {
    let mut base = key_filters();
    base.user_id = Some(String::from("alice"));
    base.include_created_by_keys = true;
    let own = equals(KeyColumn::UserId, "alice");
    let created = equals(KeyColumn::CreatedBy, "alice");
    // two arms OR together under the UI-session exclusion
    assert_eq!(
        build_key_filter_conditions(&base, now()),
        two_arm_clause(own.clone(), created.clone())
    );

    let mut no_teams = base.clone();
    no_teams.member_team_ids = Some(Vec::new());
    let empty_created = clause_of(vec![and_of(vec![
        created,
        vec![Where::IsNull(KeyColumn::TeamId)],
    ])]);
    assert_eq!(
        build_key_filter_conditions(&no_teams, now()),
        two_arm_clause(own.clone(), empty_created)
    );

    let mut current = base;
    current.member_team_ids = Some(vec![String::from("team-a"), String::from("team-b")]);
    let scoped_created = clause_of(vec![and_of(vec![
        equals(KeyColumn::CreatedBy, "alice"),
        scoped_teams(),
    ])]);
    let service_accounts = clause_of(vec![and_of(vec![
        vec![Where::In(
            KeyColumn::TeamId,
            vec![String::from("team-a"), String::from("team-b")],
        )],
        vec![Where::IsNull(KeyColumn::UserId)],
    ])]);
    // the member teams add their service accounts as a third arm next to own and created-by keys
    assert_eq!(
        build_key_filter_conditions(&current, now()),
        clause_of(vec![and_of(vec![
            filter_out_ui_session_tokens(),
            clause_of(vec![or_of(vec![own, scoped_created, service_accounts])]),
        ])])
    );
}

fn two_arm_clause(own: WhereClause, created: WhereClause) -> WhereClause {
    clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        clause_of(vec![or_of(vec![own, created])]),
    ])])
}

fn scoped_teams() -> WhereClause {
    clause_of(vec![or_of(vec![
        vec![Where::In(
            KeyColumn::TeamId,
            vec![String::from("team-a"), String::from("team-b")],
        )],
        vec![Where::IsNull(KeyColumn::TeamId)],
    ])])
}

#[test]
fn member_service_accounts_only() {
    let mut member_only = key_filters();
    member_only.member_team_ids = Some(vec![String::from("team-a")]);
    let mut entries = filter_out_ui_session_tokens();
    entries.push(and_of(vec![
        vec![Where::In(KeyColumn::TeamId, vec![String::from("team-a")])],
        vec![Where::IsNull(KeyColumn::UserId)],
    ]));
    assert_eq!(
        build_key_filter_conditions(&member_only, now()),
        clause_of(entries)
    );

    let mut overlapping = key_filters();
    overlapping.admin_team_ids = Some(vec![String::from("team-a"), String::from("team-b")]);
    overlapping.member_team_ids = Some(vec![String::from("team-a"), String::from("team-b")]);
    // every member team is already an admin team, so no service-account arm is added
    let arms = visibility_arms(&overlapping);
    assert_eq!(arms.len(), 1);
    assert_eq!(
        arms[0],
        vec![Where::In(
            KeyColumn::TeamId,
            vec![String::from("team-a"), String::from("team-b")],
        )]
    );
}

#[test]
fn no_visibility_arm() {
    let filters = key_filters();
    assert!(visibility_arms(&filters).is_empty());
    assert_eq!(
        build_key_filter_conditions(&filters, now()),
        filter_out_ui_session_tokens()
    );
}

#[test]
fn all_global_filters_intersect() {
    let mut filters = key_filters();
    filters.key_alias = Some(String::from("alias"));
    filters.key_hash = Some(String::from("hash"));
    filters.search = Some(String::from("term"));
    filters.team_id = Some(String::from("team"));
    filters.project_id = Some(String::from("project"));
    filters.access_group_id = Some(String::from("group"));
    filters.agent_id = Some(String::from("agent"));
    filters.expires_filter = Some(String::from("active"));
    filters.status_filter = Some(String::from("active"));
    let expected = clause_of(vec![and_of(vec![
        filter_out_ui_session_tokens(),
        equals(KeyColumn::KeyAlias, "alias"),
        equals(KeyColumn::Token, "hash"),
        build_key_search_where("term"),
        equals(KeyColumn::TeamId, "team"),
        equals(KeyColumn::ProjectId, "project"),
        vec![Where::HasSome(
            KeyColumn::AccessGroupIds,
            vec![String::from("group")],
        )],
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
        vec![Where::ContainsInsensitive(
            KeyColumn::KeyAlias,
            String::from("abc"),
        )],
    ])]);
    assert_eq!(build_key_search_where("abc"), expected);

    assert_eq!(
        build_key_filter_conditions(&substring_filters(), now()),
        scoped_expected(
            Where::ContainsInsensitive(KeyColumn::UserId, String::from("alice")),
            Where::ContainsInsensitive(KeyColumn::KeyAlias, String::from("Ali")),
        )
    );

    let mut exact = key_filters();
    exact.user_id = Some(String::from("alice"));
    exact.key_alias = Some(String::from("Ali"));
    assert_eq!(
        build_key_filter_conditions(&exact, now()),
        scoped_expected(
            Where::Equals(KeyColumn::UserId, text("alice")),
            Where::Equals(KeyColumn::KeyAlias, text("Ali")),
        )
    );
}

fn substring_filters() -> KeyFilters {
    let mut substring = key_filters();
    substring.user_id = Some(String::from("alice"));
    substring.key_alias = Some(String::from("Ali"));
    substring.user_matching = TextMatching::Substring;
    substring.key_alias_matching = TextMatching::Substring;
    substring
}

// the caller's own-key clause, then one global clause outside it
fn scoped_expected(user_entry: Where, alias_entry: Where) -> WhereClause {
    let mut entries = filter_out_ui_session_tokens();
    entries.push(user_entry);
    clause_of(vec![and_of(vec![clause_of(entries), vec![alias_entry]])])
}

#[test]
fn empty_filters_skip() {
    let mut filters = key_filters();
    filters.user_id = Some(String::new());
    filters.team_id = Some(String::new());
    filters.organization_id = Some(String::new());
    filters.key_alias = Some(String::new());
    filters.key_hash = Some(String::new());
    filters.exclude_team_id = Some(String::new());
    filters.admin_team_ids = Some(Vec::new());
    filters.member_team_ids = Some(Vec::new());
    filters.include_created_by_keys = true;
    filters.project_id = Some(String::new());
    filters.access_group_id = Some(String::new());
    filters.agent_id = Some(String::new());
    filters.expires_filter = Some(String::new());
    filters.search = Some(String::new());
    filters.status_filter = Some(String::new());
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
            vec![Where::IsNotNull(KeyColumn::Expires)],
            vec![Where::Before(KeyColumn::Expires, now())],
        ])])
    );
    let active_clause = build_expires_where_clause("active", now());
    assert_eq!(
        active_clause,
        clause_of(vec![or_of(vec![
            vec![Where::IsNull(KeyColumn::Expires)],
            vec![Where::AtOrAfter(KeyColumn::Expires, now())],
        ])])
    );
}

#[test]
fn blocked_null_false_true() {
    assert_eq!(status_with(None), KeyStatus::Active);
    assert_eq!(status_with(Some(false)), KeyStatus::Active);
    assert_eq!(status_with(Some(true)), KeyStatus::Revoked);

    assert_eq!(
        not_blocked_where_clause(),
        clause_of(vec![or_of(vec![
            vec![Where::IsNull(KeyColumn::Blocked)],
            vec![Where::Equals(KeyColumn::Blocked, KeyScalar::Bool(false))],
        ])])
    );
    assert_eq!(
        build_status_where_clause(Some("revoked"), now()),
        Some(vec![Where::Equals(
            KeyColumn::Blocked,
            KeyScalar::Bool(true),
        )])
    );
}

fn status_with(blocked: Option<bool>) -> KeyStatus {
    let source = KeyStatusSource {
        blocked,
        expires: None,
    };
    derive_key_status(&source, now()).unwrap()
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
    let mut filters = key_filters();
    filters.expires_filter = Some(String::from("EXPIRED"));
    filters.status_filter = Some(String::from("bogus"));
    assert_eq!(
        build_key_filter_conditions(&filters, now()),
        filter_out_ui_session_tokens()
    );
    let mut deleted = key_filters();
    deleted.status_filter = Some(String::from("deleted"));
    assert_eq!(
        build_key_filter_conditions(&deleted, now()),
        filter_out_ui_session_tokens(),
        "deleted adds no status predicate"
    );
    assert_eq!(build_status_where_clause(Some("deleted"), now()), None);
    assert_eq!(build_status_where_clause(None, now()), None);
}
