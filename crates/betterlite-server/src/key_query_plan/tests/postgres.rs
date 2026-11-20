#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::key_query_plan::filters::KeyFilters;
use crate::key_query_plan::list::list_keys;
use crate::key_query_plan::response::KeyListEntry;

use super::{key_filters, query};

const KEY_TABLE_SQL: &str = r#"
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT PRIMARY KEY,
    max_budget NUMERIC
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT,
    user_id TEXT,
    team_id TEXT,
    organization_id TEXT,
    created_by TEXT,
    project_id TEXT,
    access_group_ids TEXT[],
    agent_id TEXT,
    budget_id TEXT,
    spend NUMERIC DEFAULT 0,
    max_budget NUMERIC,
    blocked BOOLEAN,
    expires TIMESTAMP,
    created_at TIMESTAMP,
    updated_at TIMESTAMP
);
INSERT INTO "LiteLLM_VerificationToken"
    (token, key_alias, user_id, team_id, organization_id, created_by, project_id, access_group_ids, agent_id, expires, blocked)
VALUES
    ('own-alice', 'own-alice', 'alice', NULL, 'org-1', 'alice', NULL, NULL, NULL, NULL, NULL),
    ('dashboard', 'dashboard', 'alice', 'litellm-dashboard', NULL, 'alice', NULL, NULL, NULL, NULL, NULL),
    ('team-a-key', 'team-a-key', 'bob', 'team-a', NULL, 'bob', NULL, NULL, NULL, NULL, NULL),
    ('team-a-sa', 'team-a-sa', NULL, 'team-a', NULL, 'alice', NULL, NULL, NULL, NULL, NULL),
    ('team-b-key', 'team-b-key', 'bob', 'team-b', NULL, 'bob', NULL, NULL, NULL, NULL, NULL),
    ('team-b-sa', 'team-b-sa', NULL, 'team-b', NULL, 'alice', NULL, NULL, NULL, NULL, NULL),
    ('created-personal', 'created-personal', 'carol', NULL, NULL, 'alice', NULL, NULL, NULL, NULL, NULL),
    ('created-team-a', 'created-team-a', 'carol', 'team-a', NULL, 'alice', NULL, NULL, NULL, NULL, NULL),
    ('created-team-b', 'created-team-b', 'carol', 'team-b', NULL, 'alice', NULL, NULL, NULL, NULL, NULL),
    ('project-key', 'project-key', 'dave', 'team-a', NULL, 'dave', 'project-1', ARRAY['group-1'], 'agent-1', NULL, NULL),
    ('expired-key', 'expired-key', 'dave', 'team-a', NULL, 'dave', NULL, NULL, NULL, '2020-01-01T00:00:00', NULL),
    ('blocked-key', 'blocked-key', 'dave', 'team-a', NULL, 'dave', NULL, NULL, NULL, NULL, TRUE),
    ('searchable', 'Alpha Search', 'dave', 'team-a', NULL, 'dave', NULL, NULL, NULL, NULL, NULL);
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget) VALUES ('budget-1', 10);
UPDATE "LiteLLM_VerificationToken" SET budget_id = 'budget-1' WHERE token = 'own-alice';
CREATE TABLE "LiteLLM_DeletedVerificationToken" (
    id TEXT PRIMARY KEY,
    token TEXT,
    key_alias TEXT,
    user_id TEXT,
    team_id TEXT,
    deleted_at TIMESTAMP,
    deleted_by TEXT,
    deleted_by_api_key TEXT
);
INSERT INTO "LiteLLM_DeletedVerificationToken"
    (id, token, key_alias, user_id, team_id, deleted_at, deleted_by, deleted_by_api_key)
VALUES
    ('deleted-1', 'gone-1', 'gone-1', 'alice', 'team-a', '2025-01-01T00:00:00', 'admin', 'op-hash'),
    ('deleted-2', 'gone-2', 'gone-2', 'bob', NULL, '2025-01-02T00:00:00', 'admin', 'op-hash');
"#;

const METACHARACTER_SQL: &str = r#"
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT PRIMARY KEY
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT,
    spend NUMERIC DEFAULT 0,
    created_at TIMESTAMP,
    updated_at TIMESTAMP
);
INSERT INTO "LiteLLM_VerificationToken" (token, key_alias) VALUES
    ('t-pct', 'a%b'),
    ('t-x', 'axb'),
    ('t-underscore', 'a_b'),
    ('t-plain', 'ab'),
    ('t-slash', 'a\b');
CREATE TABLE "LiteLLM_DeletedVerificationToken" (
    id TEXT PRIMARY KEY,
    token TEXT
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database(sql: &str) -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["key-query-plan-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![sql.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

async fn tokens(client: PrismaClient<'_>, filters: KeyFilters) -> Vec<String> {
    let mut request = query(1, 50);
    request.filters = filters;
    let response = list_keys(Some(client), &request).await.unwrap();
    let mut found: Vec<String> = response
        .keys
        .into_iter()
        .map(|entry| match entry {
            KeyListEntry::Token(token) => token,
            KeyListEntry::Auth(_) | KeyListEntry::Deleted(_) => {
                panic!("a token-only list returns token strings")
            }
        })
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn postgres_visibility_matrix() {
    let (fixture, pool) = database(KEY_TABLE_SQL).await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };

    let counted = list_keys(Some(client), &query(1, 50)).await.unwrap();
    assert_eq!(counted.total_count, Some(12));

    let mut own_full = key_filters();
    own_full.user_id = Some(String::from("alice"));
    let mut full_request = query(1, 50);
    full_request.filters = own_full;
    full_request.return_full_object = true;
    let full_response = list_keys(Some(client), &full_request).await.unwrap();
    assert_eq!(full_response.keys.len(), 1);
    let document = serde_json::to_string(&full_response.keys[0]).unwrap();
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(&document).unwrap();
    let Some(JsonData::Object(budget)) = fields.get("litellm_budget_table") else {
        panic!("a live row carries its joined budget row");
    };
    assert_eq!(
        budget.get("budget_id"),
        Some(&JsonData::String(String::from("budget-1")))
    );

    let all = tokens(client, key_filters()).await;
    assert_eq!(
        all,
        vec![
            "blocked-key",
            "created-personal",
            "created-team-a",
            "created-team-b",
            "expired-key",
            "own-alice",
            "project-key",
            "searchable",
            "team-a-key",
            "team-a-sa",
            "team-b-key",
            "team-b-sa",
        ]
    );

    let mut own_admin_member = key_filters();
    own_admin_member.user_id = Some(String::from("alice"));
    own_admin_member.admin_team_ids = Some(vec![String::from("team-a")]);
    own_admin_member.member_team_ids = Some(vec![String::from("team-a"), String::from("team-b")]);
    let own_found = tokens(client, own_admin_member).await;
    assert_eq!(
        own_found,
        vec![
            "blocked-key",
            "expired-key",
            "own-alice",
            "project-key",
            "searchable",
            "team-a-key",
            "team-a-sa",
            "team-b-sa",
        ],
        "own keys, all admin-team keys and member-team service accounts; another member's team-b key stays hidden"
    );

    let mut unscoped_creator = key_filters();
    unscoped_creator.user_id = Some(String::from("alice"));
    unscoped_creator.include_created_by_keys = true;
    let unscoped_found = tokens(client, unscoped_creator).await;
    assert_eq!(
        unscoped_found,
        vec![
            "created-personal",
            "created-team-a",
            "created-team-b",
            "own-alice",
            "team-a-sa",
            "team-b-sa",
        ]
    );

    let mut personal_creator = key_filters();
    personal_creator.user_id = Some(String::from("alice"));
    personal_creator.member_team_ids = Some(Vec::new());
    personal_creator.include_created_by_keys = true;
    let personal_found = tokens(client, personal_creator).await;
    assert_eq!(personal_found, vec!["created-personal", "own-alice"]);

    let mut current_teams_creator = key_filters();
    current_teams_creator.user_id = Some(String::from("alice"));
    current_teams_creator.member_team_ids = Some(vec![String::from("team-a")]);
    current_teams_creator.include_created_by_keys = true;
    let current_found = tokens(client, current_teams_creator).await;
    assert_eq!(
        current_found,
        vec![
            "created-personal",
            "created-team-a",
            "own-alice",
            "team-a-sa"
        ]
    );

    let mut service_accounts = key_filters();
    service_accounts.member_team_ids = Some(vec![String::from("team-b")]);
    let service_found = tokens(client, service_accounts).await;
    assert_eq!(service_found, vec!["team-b-sa"]);

    let mut global_team = key_filters();
    global_team.team_id = Some(String::from("team-a"));
    let team_found = tokens(client, global_team).await;
    assert_eq!(
        team_found,
        vec![
            "blocked-key",
            "created-team-a",
            "expired-key",
            "project-key",
            "searchable",
            "team-a-key",
            "team-a-sa",
        ],
        "a global team filter narrows every visibility arm"
    );

    let mut globals = key_filters();
    globals.project_id = Some(String::from("project-1"));
    globals.access_group_id = Some(String::from("group-1"));
    globals.agent_id = Some(String::from("agent-1"));
    let globals_found = tokens(client, globals).await;
    assert_eq!(globals_found, vec!["project-key"]);

    let mut active = key_filters();
    active.status_filter = Some(String::from("active"));
    let active_tokens = tokens(client, active).await;
    assert!(!active_tokens.contains(&String::from("expired-key")));
    assert!(!active_tokens.contains(&String::from("blocked-key")));
    assert!(active_tokens.contains(&String::from("own-alice")));

    let mut revoked = key_filters();
    revoked.status_filter = Some(String::from("revoked"));
    assert_eq!(tokens(client, revoked).await, vec!["blocked-key"]);

    let mut search = key_filters();
    search.search = Some(String::from("alpha"));
    assert_eq!(tokens(client, search).await, vec!["searchable"]);

    let mut archived = key_filters();
    archived.status_filter = Some(String::from("deleted"));
    assert_eq!(tokens(client, archived).await, vec!["gone-1", "gone-2"]);

    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_contains_metacharacters() {
    let (fixture, pool) = database(METACHARACTER_SQL).await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };

    let mut percent = key_filters();
    percent.search = Some(String::from("a%b"));
    assert_eq!(
        tokens(client, percent).await,
        vec!["t-pct", "t-plain", "t-slash", "t-underscore", "t-x"],
        "Prisma's contains leaves % as a wildcard"
    );

    let mut underscore = key_filters();
    underscore.search = Some(String::from("a_b"));
    assert_eq!(
        tokens(client, underscore).await,
        vec!["t-pct", "t-slash", "t-underscore", "t-x"],
        "Prisma's contains leaves _ matching exactly one character"
    );

    let mut escaped = key_filters();
    escaped.search = Some(String::from("a\\%b"));
    assert_eq!(
        tokens(client, escaped).await,
        vec!["t-pct"],
        "a backslash reaches Postgres as LIKE's escape character"
    );

    pool.close();
    fixture.close_async().await.unwrap();
}
