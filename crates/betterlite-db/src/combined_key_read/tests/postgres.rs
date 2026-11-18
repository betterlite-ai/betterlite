#![cfg(test)]

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
use tokio_util::task::TaskTracker;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::BASELINE;

use crate::auth_reconnect::request::ReconnectRequest;
use crate::auth_reconnect::stale::StaleReadEngine;
use crate::database_refresh::handle::{PoolFuture, PoolTransport};
use crate::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use crate::database_replica::pinned::WriterPinnedClient;
use crate::identity_rows::key::Expiry;
use crate::key_user_writes::hash::hash_token;
use crate::team_rows::member::MemberRole;

use crate::combined_key_read::error::{DatabaseFailure, KeyReadError};
use crate::combined_key_read::grace::DeprecatedKeyCache;
use crate::combined_key_read::ports::{NoSpanResolver, PlanReconnect, ReconnectFuture};
use crate::combined_key_read::postgres::PostgresKeyDatabase;
use crate::combined_key_read::reader::{KeyReader, ReadServices};
use crate::combined_key_read::request::{CombinedKeyRequest, KeyQueryType};
use crate::combined_key_read::rows::{CACHED_PLAN_WARNING, RECONNECT_REASON};
use crate::combined_key_read::view::CombinedKeyView;

use super::harness::{
    FullJitter, ObservedDiagnostic, RecordingDiagnostics, RecordingHook, RecordingPause,
    RecordingSink, Timeline, number, text,
};

const SCHEMA: &str = r#"
ALTER TABLE "LiteLLM_VerificationToken"
    ADD COLUMN IF NOT EXISTS project_id TEXT,
    ADD COLUMN IF NOT EXISTS total_spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS tpd_limit BIGINT,
    ADD COLUMN IF NOT EXISTS object_permission_id TEXT,
    ADD COLUMN IF NOT EXISTS settings_updated_at TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS last_active TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS key_type TEXT,
    ADD COLUMN IF NOT EXISTS budget_fallbacks JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS router_settings JSONB,
    ADD COLUMN IF NOT EXISTS budget_limits JSONB;
ALTER TABLE "LiteLLM_TeamTable"
    ADD COLUMN IF NOT EXISTS soft_budget DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS tpd_limit BIGINT,
    ADD COLUMN IF NOT EXISTS object_permission_id TEXT;
ALTER TABLE "LiteLLM_BudgetTable"
    ADD COLUMN IF NOT EXISTS tpd_limit BIGINT;
ALTER TABLE "LiteLLM_TeamMembership"
    ADD COLUMN IF NOT EXISTS total_spend DOUBLE PRECISION NOT NULL DEFAULT 0.0;
CREATE TABLE IF NOT EXISTS "LiteLLM_ProjectTable" (
    "project_id" TEXT NOT NULL,
    "project_alias" TEXT,
    "metadata" JSONB NOT NULL DEFAULT '{}',
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "created_by" TEXT NOT NULL,
    "updated_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_by" TEXT NOT NULL,
    CONSTRAINT "LiteLLM_ProjectTable_pkey" PRIMARY KEY ("project_id")
);
CREATE TABLE IF NOT EXISTS "LiteLLM_DeprecatedVerificationToken" (
    "id" TEXT NOT NULL,
    "token" TEXT NOT NULL,
    "active_token_id" TEXT NOT NULL,
    "revoke_at" TIMESTAMP(3) NOT NULL,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_DeprecatedVerificationToken_pkey" PRIMARY KEY ("id"),
    CONSTRAINT "LiteLLM_DeprecatedVerificationToken_token_key" UNIQUE ("token")
);
"#;

const TOKEN_FULL: &str = "sk-full";
const TOKEN_BARE: &str = "sk-bare";
const TOKEN_MEMBER: &str = "sk-member";
const TOKEN_ORG: &str = "sk-org";

const INSERT_ORGANIZATION: &str = r#"INSERT INTO "LiteLLM_OrganizationTable"
    (organization_id, organization_alias, budget_id, metadata, created_by, updated_by)
    VALUES ($1, $2, $3, '{"o": 1}'::jsonb, 'test', 'test')"#;
const INSERT_PROJECT: &str = r#"INSERT INTO "LiteLLM_ProjectTable" (project_id, project_alias, metadata, created_by, updated_by)
    VALUES ('proj-1', 'Project One', '{"p": 1}'::jsonb, 'test', 'test')"#;
const INSERT_MEMBERSHIP: &str = r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, budget_id)
    VALUES ($1, 'team-1', $2, $3)"#;
const INSERT_FULL_KEY: &str = r#"INSERT INTO "LiteLLM_VerificationToken"
    (token, user_id, team_id, organization_id, project_id, budget_id, models,
     model_max_budget, expires)
    VALUES ($1, 'u1', 'team-1', 'org-key', 'proj-1', 'budget-key', ARRAY[]::TEXT[],
     '{}'::jsonb, '2030-01-02 03:04:05')"#;
const INSERT_BARE_KEY: &str =
    r#"INSERT INTO "LiteLLM_VerificationToken" (token, models) VALUES ($1, ARRAY[]::TEXT[])"#;
const INSERT_LIVE_DEPRECATED_ROW: &str = r#"INSERT INTO "LiteLLM_DeprecatedVerificationToken"
    (id, token, active_token_id, revoke_at)
    VALUES ($1, $2, $3, '2030-01-01 00:00:00')"#;
const INSERT_EXPIRED_DEPRECATED_ROW: &str = r#"INSERT INTO "LiteLLM_DeprecatedVerificationToken"
    (id, token, active_token_id, revoke_at)
    VALUES ($1, $2, $3, '2020-01-01 00:00:00')"#;

// Postgres has no SELECT trigger, so the joined team table becomes a view whose predicate raises a chosen server message
const RAISE_CACHED_PLAN: &str = r"
CREATE FUNCTION raise_select_error() RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'cached plan must not change result type';
END
$$";
const RAISE_ORDINARY_ERROR: &str = r"
CREATE OR REPLACE FUNCTION raise_select_error() RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'relation disappeared';
END
$$";
const RENAME_TEAM_TABLE: &str =
    r#"ALTER TABLE "LiteLLM_TeamTable" RENAME TO "LiteLLM_TeamTable_base""#;
const CREATE_TEAM_VIEW: &str = r#"
CREATE VIEW "LiteLLM_TeamTable" AS
    SELECT * FROM "LiteLLM_TeamTable_base" WHERE raise_select_error()
"#;
const INSERT_GUARDED_TEAM: &str = r#"INSERT INTO "LiteLLM_TeamTable_base" (team_id, team_alias, members_with_roles)
    VALUES ('team-native', 'Team Native', '[]'::jsonb)"#;
const INSERT_GUARDED_KEY: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, team_id, models)
    VALUES ($1, 'team-native', ARRAY[]::TEXT[])"#;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct TestClient {
    pool: Pool,
}

impl DatabaseLifecycle for TestClient {
    type Error = Infallible;

    fn is_connected(&self) -> bool {
        true
    }

    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(true) })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for TestClient {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        let known = matches!(
            name,
            "query_first" | "litellm_deprecatedverificationtoken" | "find_first"
        );
        Ok(known.then(|| Attribute::Model(self.pool.clone())))
    }

    fn token_refresh_enabled(&self) -> bool {
        false
    }

    fn token_label(&self) -> &'static str {
        "database token"
    }

    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}

#[derive(Default)]
struct ObservedReconnects {
    attempts: Mutex<Vec<(String, bool)>>,
}

impl PlanReconnect for ObservedReconnects {
    type Handle = ();

    fn read_target(&self) -> crate::database_refresh::generation::Observation<()> {
        crate::database_refresh::generation::Observation {
            generation: 0,
            handle: None,
            connected: true,
        }
    }

    fn attempt<'reconnect>(
        &'reconnect self,
        request: &'reconnect ReconnectRequest<'_>,
        _stale: Option<&'reconnect StaleReadEngine<Self::Handle>>,
    ) -> ReconnectFuture<'reconnect> {
        self.attempts
            .lock()
            .unwrap()
            .push((request.reason.to_owned(), request.force_recreate));
        Box::pin(async { Ok(true) })
    }
}

pub(crate) struct Fixture {
    fixture: PostgresFixture,
    pool: Pool,
    client: TestClient,
}

impl Fixture {
    pub(crate) async fn start() -> Self {
        let configured = Environment::parse_from(["combined-key-read-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            configured.as_ref(),
            vec![BASELINE.to_owned(), SCHEMA.to_owned()],
        )
        .resolve()
        .await
        .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(4)
            .build()
            .unwrap();
        Self {
            fixture,
            pool: pool.clone(),
            client: TestClient { pool },
        }
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }

    async fn execute(&self, sql: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.unwrap();
        client.execute(sql, parameters).await.unwrap();
    }

    pub(crate) async fn read(&self, token: &str) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let timeline = Timeline::default();
        let diagnostics = RecordingDiagnostics::shared(&timeline);
        let hook = RecordingHook::shared(&timeline);
        let pause = RecordingPause::shared(&timeline);
        let sink = Arc::new(RecordingSink::shared(&timeline));
        let tasks = TaskTracker::new();
        let database = PostgresKeyDatabase {
            database: Some(WriterPinnedClient::Writer(&self.client)),
        };
        let reconnect = ObservedReconnects::default();
        let reader = KeyReader {
            database: &database,
            reconnect: &reconnect,
            sink,
            services: ReadServices {
                clock: &crate::key_user_writes::retry::SystemWriteClock,
                jitter: &FullJitter,
                pause: &pause,
                diagnostics: &diagnostics,
                hook: &hook,
                resolver: &NoSpanResolver,
                tasks: &tasks,
            },
            cache: Arc::new(Mutex::new(DeprecatedKeyCache::default())),
        };
        let request = CombinedKeyRequest {
            token: Some(SecretString::from(token)),
            query_type: KeyQueryType::FindUnique,
            check_deprecated: true,
            use_writer: false,
            parent_otel_span: None,
        };
        let outcome = reader.read(&request).await;
        assert!(reconnect.attempts.lock().unwrap().is_empty());
        outcome
    }
}

async fn read_native(
    fixture: &Fixture,
    reconnect: &ObservedReconnects,
    token: &str,
) -> (
    Result<Option<CombinedKeyView>, KeyReadError>,
    Vec<ObservedDiagnostic>,
) {
    let timeline = Timeline::default();
    let diagnostics = RecordingDiagnostics::shared(&timeline);
    let hook = RecordingHook::shared(&timeline);
    let pause = RecordingPause::shared(&timeline);
    let sink = Arc::new(RecordingSink::shared(&timeline));
    let tasks = TaskTracker::new();
    let database = PostgresKeyDatabase {
        database: Some(WriterPinnedClient::Writer(&fixture.client)),
    };
    let reader = KeyReader {
        database: &database,
        reconnect,
        sink,
        services: ReadServices {
            clock: &crate::key_user_writes::retry::SystemWriteClock,
            jitter: &FullJitter,
            pause: &pause,
            diagnostics: &diagnostics,
            hook: &hook,
            resolver: &NoSpanResolver,
            tasks: &tasks,
        },
        cache: Arc::new(Mutex::new(DeprecatedKeyCache::default())),
    };
    let request = CombinedKeyRequest {
        token: Some(SecretString::from(token)),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: false,
        use_writer: false,
        parent_otel_span: None,
    };
    let outcome = reader.read(&request).await;
    (outcome, diagnostics.seen())
}

pub(crate) async fn every_join_alias_and_null(fixture: &Fixture) {
    seed_budgets(fixture).await;
    fixture
        .execute(
            INSERT_ORGANIZATION,
            &[&"org-team", &"Team Org", &"budget-org"],
        )
        .await;
    fixture
        .execute(
            INSERT_ORGANIZATION,
            &[&"org-key", &"Org Key", &"budget-org"],
        )
        .await;
    let model_id = seed_model(fixture).await;
    seed_team(fixture, model_id).await;
    fixture.execute(INSERT_PROJECT, &[]).await;
    fixture
        .execute(INSERT_MEMBERSHIP, &[&"u1", &3.0, &"budget-tm"])
        .await;
    let stored_full = hash_token(TOKEN_FULL);
    fixture.execute(INSERT_FULL_KEY, &[&stored_full]).await;
    let stored_bare = hash_token(TOKEN_BARE);
    fixture.execute(INSERT_BARE_KEY, &[&stored_bare]).await;
    let view = fixture.read(TOKEN_FULL).await.unwrap().unwrap();
    assert_team_fields(&view);
    assert_budget_fields(&view);
    assert_org_and_project_fields(&view);
    let bare = fixture.read(TOKEN_BARE).await.unwrap().unwrap();
    assert_null_joins(&bare);
}

pub(crate) async fn membership_requires_both_ids(fixture: &Fixture) {
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, tpm_limit, rpm_limit, created_by, updated_by)
               VALUES ('budget-m2', 55, 56, 'test', 'test')"#,
            &[],
        )
        .await;
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, spend, members_with_roles)
               VALUES ('team-2', 'Team Two', 0.0, '[]'::jsonb)"#,
            &[],
        )
        .await;
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, budget_id)
               VALUES ('u2', 'team-2', 9.0, 'budget-m2')"#,
            &[],
        )
        .await;
    let stored = hash_token(TOKEN_MEMBER);
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id, team_id, models)
               VALUES ($1, 'u1', 'team-2', ARRAY[]::TEXT[])"#,
            &[&stored],
        )
        .await;
    let without = fixture.read(TOKEN_MEMBER).await.unwrap().unwrap();
    assert!(without.team_member.is_none());
    assert!(without.team_member_spend.is_none());
    assert!(without.team_member_tpm_limit.is_none());
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, budget_id)
               VALUES ('u1', 'team-2', 4.0, 'budget-m2')"#,
            &[],
        )
        .await;
    let with = fixture.read(TOKEN_MEMBER).await.unwrap().unwrap();
    assert_eq!(with.team_member_spend, Some(rust_decimal::Decimal::from(4)));
    assert_eq!(with.team_member_tpm_limit, Some(55));
    assert_eq!(with.team_member_rpm_limit, Some(56));
}

pub(crate) async fn key_org_differs_from_team_org(fixture: &Fixture) {
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by)
               VALUES ('budget-team-org', 11, 'test', 'test')"#,
            &[],
        )
        .await;
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by)
               VALUES ('budget-key-org', 22, 'test', 'test')"#,
            &[],
        )
        .await;
    fixture
        .execute(
            INSERT_ORGANIZATION,
            &[&"org-team", &"Team Org", &"budget-team-org"],
        )
        .await;
    fixture
        .execute(
            INSERT_ORGANIZATION,
            &[&"org-key", &"Key Org", &"budget-key-org"],
        )
        .await;
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, organization_id, members_with_roles)
               VALUES ('team-3', 'Team Three', 'org-team', '[]'::jsonb)"#,
            &[],
        )
        .await;
    let stored = hash_token(TOKEN_ORG);
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id, team_id, organization_id, models)
               VALUES ($1, 'u1', 'team-3', 'org-key', ARRAY[]::TEXT[])"#,
            &[&stored],
        )
        .await;
    let view = fixture.read(TOKEN_ORG).await.unwrap().unwrap();
    assert_eq!(view.token.org_id.as_deref(), Some("org-key"));
    assert_eq!(view.organization_alias.as_deref(), Some("Key Org"));
    assert_eq!(
        view.organization_max_budget,
        Some(rust_decimal::Decimal::from(22))
    );
    assert_eq!(view.team_alias.as_deref(), Some("Team Three"));
}

pub(crate) async fn native_driver_error_message_is_classified(fixture: &Fixture) {
    fixture.execute(RAISE_CACHED_PLAN, &[]).await;
    fixture.execute(RENAME_TEAM_TABLE, &[]).await;
    fixture.execute(CREATE_TEAM_VIEW, &[]).await;
    fixture.execute(INSERT_GUARDED_TEAM, &[]).await;
    let stored = hash_token("sk-native-plan");
    fixture.execute(INSERT_GUARDED_KEY, &[&stored]).await;
    let reconnect = ObservedReconnects::default();
    let (outcome, diagnostics) = read_native(fixture, &reconnect, "sk-native-plan").await;
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        KeyReadError::Database(DatabaseFailure::Reported(_))
    ));
    assert!(
        diagnostics.contains(&ObservedDiagnostic::CachedPlan(String::from(
            CACHED_PLAN_WARNING
        )))
    );
    let attempts = reconnect.attempts.lock().unwrap();
    assert_eq!(attempts.len(), 3);
    assert!(
        attempts
            .iter()
            .all(|(reason, force_recreate)| reason == RECONNECT_REASON && *force_recreate)
    );
}

pub(crate) async fn ordinary_driver_error_is_not_a_cached_plan(fixture: &Fixture) {
    fixture.execute(RAISE_CACHED_PLAN, &[]).await;
    fixture.execute(RENAME_TEAM_TABLE, &[]).await;
    fixture.execute(CREATE_TEAM_VIEW, &[]).await;
    fixture.execute(INSERT_GUARDED_TEAM, &[]).await;
    fixture.execute(RAISE_ORDINARY_ERROR, &[]).await;
    let stored = hash_token("sk-native-plain");
    fixture.execute(INSERT_GUARDED_KEY, &[&stored]).await;
    let reconnect = ObservedReconnects::default();
    let (outcome, diagnostics) = read_native(fixture, &reconnect, "sk-native-plain").await;
    assert!(matches!(outcome.unwrap_err(), KeyReadError::Database(_)));
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, ObservedDiagnostic::CachedPlan(_)))
    );
    assert!(reconnect.attempts.lock().unwrap().is_empty());
}

pub(crate) async fn deprecated_rotation_reads_the_active_key(fixture: &Fixture) {
    let active = hash_token("sk-active-rotation");
    fixture.execute(INSERT_BARE_KEY, &[&active]).await;
    let deprecated = hash_token("sk-old-rotation");
    fixture
        .execute(
            INSERT_LIVE_DEPRECATED_ROW,
            &[&"dep-live", &deprecated.as_str(), &active.as_str()],
        )
        .await;
    let view = fixture.read("sk-old-rotation").await.unwrap().unwrap();
    assert_eq!(view.token.token.unwrap().expose_secret(), active.as_str());
    let expired = hash_token("sk-expired-rotation");
    fixture
        .execute(
            INSERT_EXPIRED_DEPRECATED_ROW,
            &[&"dep-expired", &expired.as_str(), &active.as_str()],
        )
        .await;
    assert!(fixture.read("sk-expired-rotation").await.unwrap().is_none());
}

async fn seed_budgets(fixture: &Fixture) {
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable"
               (budget_id, max_budget, soft_budget, tpm_limit, rpm_limit, tpd_limit, model_max_budget, created_by, updated_by)
               VALUES ('budget-key', 100, 5, 21, 22, 23, '{"m": 5}'::jsonb, 'test', 'test')"#,
            &[],
        )
        .await;
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, tpm_limit, rpm_limit, created_by, updated_by)
               VALUES ('budget-tm', 11, 12, 'test', 'test')"#,
            &[],
        )
        .await;
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, tpm_limit, rpm_limit, created_by, updated_by)
               VALUES ('budget-org', 200, 31, 32, 'test', 'test')"#,
            &[],
        )
        .await;
}

async fn seed_model(fixture: &Fixture) -> i32 {
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_ModelTable" (aliases, created_by, updated_by)
               VALUES ('{"alias": "model"}'::jsonb, 'test', 'test')"#,
            &[],
        )
        .await;
    let client = fixture.pool.get().await.unwrap();
    let row = client
        .query_one(r#"SELECT id FROM "LiteLLM_ModelTable" LIMIT 1"#, &[])
        .await
        .unwrap();
    row.get(0)
}

async fn seed_team(fixture: &Fixture, model_id: i32) {
    fixture
        .execute(
            r#"INSERT INTO "LiteLLM_TeamTable"
               (team_id, team_alias, organization_id, spend, max_budget, soft_budget, tpm_limit, rpm_limit,
                tpd_limit, models, metadata, blocked, members_with_roles, object_permission_id, model_max_budget, model_id)
               VALUES ('team-1', 'Team One', 'org-team', 10, 50, 7, 101, 102,
                103, ARRAY['gpt-4'], '{"t": 1}'::jsonb, true,
                '[{"user_id": "u1", "role": "admin", "user_email": "a@x"}]'::jsonb, 'tm-perm', '{"tm": 1}'::jsonb, $1)"#,
            &[&model_id],
        )
        .await;
}

fn assert_team_fields(view: &CombinedKeyView) {
    assert_eq!(view.team_spend, Some(rust_decimal::Decimal::from(10)));
    assert_eq!(view.team_alias.as_deref(), Some("Team One"));
    assert_eq!(view.team_tpm_limit, Some(101));
    assert_eq!(view.team_rpm_limit, Some(102));
    assert_eq!(view.team_tpd_limit, Some(103));
    assert_eq!(view.team_max_budget, Some(rust_decimal::Decimal::from(50)));
    assert_eq!(view.team_soft_budget, Some(rust_decimal::Decimal::from(7)));
    assert_eq!(view.team_models.len(), 1);
    assert_eq!(view.team_models[0], text("gpt-4"));
    assert!(view.team_blocked);
    assert_eq!(
        view.team_metadata.as_ref().unwrap().0.get("t"),
        Some(&number(1))
    );
    assert_eq!(view.team_object_permission_id.as_deref(), Some("tm-perm"));
    assert_eq!(
        view.team_model_aliases.as_ref().unwrap().0.get("alias"),
        Some(&text("model"))
    );
    assert_eq!(view.team_member_spend, Some(rust_decimal::Decimal::from(3)));
    assert_eq!(view.team_member_tpm_limit, Some(11));
    assert_eq!(view.team_member_rpm_limit, Some(12));
    let member = view.team_member.as_ref().unwrap();
    assert_eq!(member.base.user_id.as_deref(), Some("u1"));
    assert_eq!(member.role, MemberRole::Admin);
}

fn assert_budget_fields(view: &CombinedKeyView) {
    assert_eq!(
        view.token.max_budget,
        Some(rust_decimal::Decimal::from(100))
    );
    assert_eq!(view.soft_budget, Some(rust_decimal::Decimal::from(5)));
    assert_eq!(view.token.tpm_limit, Some(21));
    assert_eq!(view.token.rpm_limit, Some(22));
    assert_eq!(view.token.tpd_limit, Some(23));
    assert_eq!(view.token.model_max_budget.0.get("m"), Some(&number(5)));
    assert!(view.last_refreshed_at.is_some());
    assert_eq!(
        view.token.expires,
        Some(Expiry::Text(String::from("2030-01-02T03:04:05")))
    );
}

fn assert_org_and_project_fields(view: &CombinedKeyView) {
    assert_eq!(view.token.org_id.as_deref(), Some("org-key"));
    assert_eq!(view.organization_alias.as_deref(), Some("Org Key"));
    assert_eq!(
        view.organization_max_budget,
        Some(rust_decimal::Decimal::from(200))
    );
    assert_eq!(view.organization_tpm_limit, Some(31));
    assert_eq!(view.organization_rpm_limit, Some(32));
    assert_eq!(
        view.organization_metadata.as_ref().unwrap().0.get("o"),
        Some(&number(1))
    );
    assert_eq!(view.project_alias.as_deref(), Some("Project One"));
    assert_eq!(
        view.project_metadata.as_ref().unwrap().0.get("p"),
        Some(&number(1))
    );
}

fn assert_null_joins(view: &CombinedKeyView) {
    assert!(view.team_spend.is_none());
    assert!(view.team_alias.is_none());
    assert!(view.team_max_budget.is_none());
    assert!(view.team_models.is_empty());
    assert!(!view.team_blocked);
    assert!(view.team_member.is_none());
    assert!(view.project_alias.is_none());
    assert!(view.organization_alias.is_none());
    assert!(view.token.org_id.is_none());
    assert!(view.token.max_budget.is_none());
    assert!(view.soft_budget.is_none());
    assert!(view.last_refreshed_at.is_some());
}
