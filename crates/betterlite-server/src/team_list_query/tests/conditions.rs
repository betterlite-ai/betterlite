use std::sync::Arc;

use crate::team_list_query::conditions::{
    TeamListColumn, TeamListScope, TeamListWhere, build_where_conditions,
};

use super::support::{
    FailingStore, RecordingCache, database, deleted_team_insert, lookup, organization_insert,
    rejected, searching, team_filters, team_insert, user_insert, user_store,
};

fn scope_seed() -> String {
    let mut seed = organization_insert("org-1");
    seed.push_str(&organization_insert("org-2"));
    seed.push_str(&team_insert(
        "in-scope",
        "gamma team",
        Some("org-1"),
        &[],
        &[],
    ));
    seed.push_str(&team_insert(
        "out-of-scope",
        "gamma other",
        Some("org-2"),
        &[],
        &[],
    ));
    seed.push_str(&team_insert(
        "own-outside",
        "nothing",
        Some("org-2"),
        &[],
        &[],
    ));
    seed
}

#[tokio::test]
async fn exact_and_prefix_search() {
    let seed = [
        organization_insert("org-1"),
        team_insert("alpha", "First", Some("org-1"), &[], &[]),
        team_insert("alpha-2", "Second", Some("org-1"), &[], &[]),
        team_insert("beta", "Alpha Team", Some("org-1"), &[], &[]),
        team_insert("ALPHA-3", "Third", Some("org-1"), &[], &[]),
    ]
    .concat();
    let fixture = database(&seed).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let exact = searching("alpha", false);
    let clause = build_where_conditions(&no_database, &exact, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
    let prefix_filters = searching("ALPHA", true);
    let prefix_clause =
        build_where_conditions(&no_database, &prefix_filters, TeamListScope::default())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&prefix_clause, false).await,
        vec!["ALPHA-3".to_owned(), "beta".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn insensitive_alias() {
    let seed = [
        organization_insert("org-1"),
        team_insert("t1", "Alpha One", Some("org-1"), &[], &[]),
        team_insert("t2", "beta", Some("org-1"), &[], &[]),
        team_insert("t3", "gamma", Some("org-1"), &[], &[]),
        team_insert("t4", "The Alpha One", Some("org-1"), &[], &[]),
    ]
    .concat();
    let fixture = database(&seed).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let filters = team_filters(None, Some("aLp"), None, None);
    let clause = build_where_conditions(&no_database, &filters, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["t1".to_owned(), "t4".to_owned()]
    );
    fixture.close().await;
}

fn wildcard_seed() -> String {
    [
        organization_insert("org-1"),
        team_insert("t-pct", "a%b", Some("org-1"), &[], &[]),
        team_insert("t-x", "axb", Some("org-1"), &[], &[]),
        team_insert("t-underscore", "a_b", Some("org-1"), &[], &[]),
        team_insert("t-plain", "ab", Some("org-1"), &[], &[]),
        team_insert("t-slash", "a\\b", Some("org-1"), &[], &[]),
    ]
    .concat()
}

#[tokio::test]
async fn alias_wildcards_are_patterns() {
    let fixture = database(&wildcard_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    // Prisma's contains reaches Postgres unescaped, so a percent matches any run of characters
    let percent = team_filters(None, Some("a%b"), None, None);
    let percent_clause = build_where_conditions(&no_database, &percent, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&percent_clause, false).await,
        vec![
            "t-pct".to_owned(),
            "t-plain".to_owned(),
            "t-slash".to_owned(),
            "t-underscore".to_owned(),
            "t-x".to_owned(),
        ]
    );
    // an underscore matches exactly one character, so the alias without one in between drops out
    let underscore = team_filters(None, Some("a_b"), None, None);
    let underscore_clause =
        build_where_conditions(&no_database, &underscore, TeamListScope::default())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&underscore_clause, false).await,
        vec![
            "t-pct".to_owned(),
            "t-slash".to_owned(),
            "t-underscore".to_owned(),
            "t-x".to_owned(),
        ]
    );
    // a backslash is LIKE's escape character, so \% finds the alias holding a percent sign
    let escaped = team_filters(None, Some(r"a\%b"), None, None);
    let escaped_clause = build_where_conditions(&no_database, &escaped, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&escaped_clause, false).await,
        vec!["t-pct".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn prefix_wildcards_are_patterns() {
    let fixture = database(&wildcard_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    // a prefix search is an exact match on team_id and a contains on the alias, both unescaped
    let prefixed = searching("t-pc_", true);
    let prefix_clause = build_where_conditions(&no_database, &prefixed, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&prefix_clause, false).await,
        vec!["t-pct".to_owned()]
    );
    let percent = searching("t-p%", true);
    let percent_clause = build_where_conditions(&no_database, &percent, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&percent_clause, false).await,
        vec!["t-pct".to_owned(), "t-plain".to_owned()]
    );
    let alias = searching("a_", true);
    let alias_clause = build_where_conditions(&no_database, &alias, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&alias_clause, false).await,
        vec![
            "t-pct".to_owned(),
            "t-slash".to_owned(),
            "t-underscore".to_owned(),
            "t-x".to_owned(),
        ]
    );
    let escaped = searching(r"a\%", true);
    let escaped_clause = build_where_conditions(&no_database, &escaped, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&escaped_clause, false).await,
        vec!["t-pct".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn search_and_scope_intersection() {
    let fixture = database(&scope_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let filters = searching("gamma", false);
    let administered = vec!["org-1".to_owned()];
    let own = vec!["own-outside".to_owned()];
    let scope = TeamListScope {
        org_admin_org_ids: Some(&administered),
        own_team_ids: &own,
    };
    let clause = build_where_conditions(&no_database, &filters, scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["in-scope".to_owned()]
    );
    // without a search the union keeps the caller's own team outside the administered orgs
    let unfiltered = team_filters(None, None, None, None);
    let union_clause = build_where_conditions(&no_database, &unfiltered, scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&union_clause, false).await,
        vec!["in-scope".to_owned(), "own-outside".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn org_scope_without_own_teams() {
    let fixture = database(&scope_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let filters = team_filters(None, None, None, None);
    let administered = vec!["org-1".to_owned()];
    // a caller with no own team in the database reads the administered orgs alone
    let scope = TeamListScope {
        org_admin_org_ids: Some(&administered),
        own_team_ids: &[],
    };
    let clause = build_where_conditions(&no_database, &filters, scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["in-scope".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn explicit_org_wins() {
    let fixture = database(&scope_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let filters = team_filters(None, None, Some("org-2"), None);
    let administered = vec!["org-1".to_owned()];
    let own = vec!["own-outside".to_owned()];
    let scope = TeamListScope {
        org_admin_org_ids: Some(&administered),
        own_team_ids: &own,
    };
    let clause = build_where_conditions(&no_database, &filters, scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["out-of-scope".to_owned(), "own-outside".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn deleted_members_has() {
    let seed = [
        deleted_team_insert("d1", &["u1"]),
        deleted_team_insert("d2", &["u2"]),
        user_insert("u1", &[]),
    ]
    .concat();
    let fixture = database(&seed).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let mut filters = team_filters(None, None, None, Some("u1"));
    filters.use_deleted_table = true;
    let clause = build_where_conditions(
        &lookup(&cache, Some(store)),
        &filters,
        TeamListScope::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        clause,
        Some(vec![TeamListWhere::MembersHas("u1".to_owned())])
    );
    assert_eq!(
        fixture
            .matching_team_ids(clause.as_ref().unwrap(), true)
            .await,
        vec!["d1".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn empty_membership_skips_query() {
    let fixture = database(&user_insert("u1", &[])).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let plain = team_filters(None, None, None, Some("u1"));
    assert_eq!(
        build_where_conditions(&users, &plain, TeamListScope::default())
            .await
            .unwrap(),
        None
    );
    // the skip happens before the exact team_id check, so even a named team is not queried
    let named = team_filters(Some("t1"), None, None, Some("u1"));
    assert_eq!(
        build_where_conditions(&users, &named, TeamListScope::default())
            .await
            .unwrap(),
        None
    );
    fixture.close().await;
}

#[tokio::test]
async fn user_membership_in_filter() {
    let seed = [
        organization_insert("org-1"),
        team_insert("t1", "one", Some("org-1"), &[], &[]),
        team_insert("t2", "two", Some("org-1"), &[], &[]),
        team_insert("t3", "three", Some("org-1"), &[], &[]),
        user_insert("u1", &["t1", "t3"]),
    ]
    .concat();
    let fixture = database(&seed).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let filters = team_filters(None, None, None, Some("u1"));
    let clause = build_where_conditions(&users, &filters, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        clause,
        vec![TeamListWhere::In(
            TeamListColumn::TeamId,
            vec!["t1".to_owned(), "t3".to_owned()]
        )]
    );
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["t1".to_owned(), "t3".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn empty_string_filters_are_unset() {
    let cache = Arc::new(RecordingCache::default());
    let no_database = lookup(&cache, None);
    let filters = team_filters(Some(""), Some(""), Some(""), None);
    assert_eq!(
        build_where_conditions(&no_database, &filters, TeamListScope::default())
            .await
            .unwrap(),
        Some(Vec::new())
    );
}

#[tokio::test]
async fn nonmember_team_404() {
    let fixture = database(&user_insert("u1", &["t1"])).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let foreign = team_filters(Some("t9"), None, None, Some("u1"));
    let rejection =
        rejected(build_where_conditions(&users, &foreign, TeamListScope::default()).await);
    assert_eq!(rejection.status(), 404);
    assert_eq!(
        rejection.detail().error,
        "User is not a member of team_id=t9"
    );
    // the check reads team_id is not None, not its truthiness, so an empty ID is still checked
    let empty = team_filters(Some(""), None, None, Some("u1"));
    let empty_rejection =
        rejected(build_where_conditions(&users, &empty, TeamListScope::default()).await);
    assert_eq!(empty_rejection.status(), 404);
    let member = team_filters(Some("t1"), None, None, Some("u1"));
    let clause = build_where_conditions(&users, &member, TeamListScope::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        clause,
        vec![TeamListWhere::Equals(
            TeamListColumn::TeamId,
            "t1".to_owned()
        )]
    );
    fixture.close().await;
}

#[tokio::test]
async fn missing_user_404() {
    let fixture = database(&user_insert("u1", &["t1"])).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let ghost = team_filters(None, None, None, Some("ghost"));
    let rejection =
        rejected(build_where_conditions(&users, &ghost, TeamListScope::default()).await);
    assert_eq!(rejection.status(), 404);
    assert_eq!(
        rejection.detail().error,
        "User not found, passed user_id=ghost"
    );
    // an ordinary read failure is the same ValueError and the same 404
    let failing = lookup(&cache, Some(Arc::new(FailingStore)));
    let unreadable_rejection =
        rejected(build_where_conditions(&failing, &ghost, TeamListScope::default()).await);
    assert_eq!(unreadable_rejection.status(), 404);
    assert_eq!(
        unreadable_rejection.detail().error,
        "User not found, passed user_id=ghost"
    );
    fixture.close().await;
}
