use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::jwt::JwtMappingSource;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::BASELINE;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Config, NoTls};

use super::support;
use crate::access_group_sync::team::TEAM_ADVISORY_LOCK_SQL;
use crate::audit_events::identity::CallerIdentity;
use crate::bulk_team_removal::error::BulkRemovalError;
use crate::bulk_team_removal::postgres::{BATCH_TX_TIMEOUT, PostgresTeamRemoval};
use crate::bulk_team_removal::removal::{TeamRemoval, TeamRemovalSession};
use crate::bulk_team_removal::service::BulkTeamRemoval;
use crate::key_archive::clock::{ArchiveClock, SystemArchiveClock};
use crate::team_request_contracts::bulk_members::TeamMemberDeleteResult;

const ROSTER_U1: &str = r#"[{"user_id":"u1","role":"user"}]"#;
const ROSTER_U2: &str = r#"[{"user_id":"u2","role":"user"}]"#;
const ROSTER_U1_U2: &str = r#"[{"user_id":"u1","role":"user"},{"user_id":"u2","role":"user"}]"#;
const ROSTER_U1_U2_U3: &str = r#"[{"user_id":"u1","role":"user"},{"user_id":"u2","role":"user"},{"user_id":"u3","role":"user"}]"#;
const EMAIL_ROSTER: &str = r#"[{"user_email":"a@example.com","role":"user"}]"#;

// the archive table B10129 writes, so the trigger can prove the key row is archived before it is deleted
const DELETED_TOKENS: &str = r#"CREATE TABLE IF NOT EXISTS "LiteLLM_DeletedVerificationToken" (
    "id" TEXT NOT NULL,
    "token" TEXT NOT NULL,
    "key_name" TEXT,
    "key_alias" TEXT,
    "soft_budget_cooldown" BOOLEAN NOT NULL DEFAULT false,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "expires" TIMESTAMP(3),
    "models" TEXT[],
    "aliases" JSONB NOT NULL DEFAULT '{}',
    "config" JSONB NOT NULL DEFAULT '{}',
    "user_id" TEXT,
    "team_id" TEXT,
    "agent_id" TEXT,
    "project_id" TEXT,
    "permissions" JSONB NOT NULL DEFAULT '{}',
    "max_parallel_requests" INTEGER,
    "metadata" JSONB NOT NULL DEFAULT '{}',
    "blocked" BOOLEAN,
    "tpm_limit" BIGINT,
    "rpm_limit" BIGINT,
    "tpd_limit" BIGINT,
    "max_budget" DOUBLE PRECISION,
    "budget_duration" TEXT,
    "budget_reset_at" TIMESTAMP(3),
    "allowed_cache_controls" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "allowed_routes" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "key_type" TEXT,
    "policies" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "access_group_ids" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "model_spend" JSONB NOT NULL DEFAULT '{}',
    "model_max_budget" JSONB NOT NULL DEFAULT '{}',
    "budget_fallbacks" JSONB NOT NULL DEFAULT '{}',
    "router_settings" JSONB DEFAULT '{}',
    "budget_id" TEXT,
    "organization_id" TEXT,
    "object_permission_id" TEXT,
    "created_at" TIMESTAMP(3),
    "created_by" TEXT,
    "updated_at" TIMESTAMP(3),
    "updated_by" TEXT,
    "settings_updated_at" TIMESTAMP(3),
    "last_active" TIMESTAMP(3),
    "rotation_count" INTEGER DEFAULT 0,
    "auto_rotate" BOOLEAN DEFAULT false,
    "rotation_interval" TEXT,
    "last_rotation_at" TIMESTAMP(3),
    "key_rotation_at" TIMESTAMP(3),
    "deleted_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "deleted_by" TEXT,
    "deleted_by_api_key" TEXT,
    "litellm_changed_by" TEXT,
    CONSTRAINT "LiteLLM_DeletedVerificationToken_pkey" PRIMARY KEY ("id")
)"#;

// a named stage in test_failures makes one ordered write raise, so a rollback is read back as the pre-removal state
const TRIGGERS: &str = r#"CREATE TABLE IF NOT EXISTS test_failures ("stage" TEXT NOT NULL);
CREATE OR REPLACE FUNCTION test_fail_update() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM test_failures WHERE "stage" = TG_ARGV[0]) THEN
        RAISE EXCEPTION 'injected failure at %', TG_ARGV[0];
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE OR REPLACE FUNCTION test_fail_delete() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM test_failures WHERE "stage" = TG_ARGV[0]) THEN
        RAISE EXCEPTION 'injected failure at %', TG_ARGV[0];
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE OR REPLACE FUNCTION test_archived_first() RETURNS TRIGGER AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM "LiteLLM_DeletedVerificationToken" WHERE "token" = OLD."token") THEN
        RAISE EXCEPTION 'key % was deleted before it was archived', OLD."token";
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER test_fail_roster BEFORE UPDATE ON "LiteLLM_TeamTable" FOR EACH ROW EXECUTE FUNCTION test_fail_update('roster');
CREATE TRIGGER test_fail_user_teams BEFORE UPDATE ON "LiteLLM_UserTable" FOR EACH ROW EXECUTE FUNCTION test_fail_update('user-teams');
CREATE TRIGGER test_fail_membership BEFORE DELETE ON "LiteLLM_TeamMembership" FOR EACH ROW EXECUTE FUNCTION test_fail_delete('membership');
CREATE TRIGGER test_fail_archive BEFORE INSERT ON "LiteLLM_DeletedVerificationToken" FOR EACH ROW EXECUTE FUNCTION test_fail_update('archive');
CREATE TRIGGER test_fail_key_delete BEFORE DELETE ON "LiteLLM_VerificationToken" FOR EACH ROW EXECUTE FUNCTION test_fail_delete('key-delete');
CREATE TRIGGER test_require_archived BEFORE DELETE ON "LiteLLM_VerificationToken" FOR EACH ROW EXECUTE FUNCTION test_archived_first();"#;

const INSERT_TEAM: &str = r#"INSERT INTO "LiteLLM_TeamTable" ("team_id", "members_with_roles") VALUES ($1, to_jsonb($2::TEXT))"#;
const SET_ROSTER: &str = r#"UPDATE "LiteLLM_TeamTable" SET "members_with_roles" = to_jsonb($2::TEXT) WHERE "team_id" = $1"#;
const DELETE_TEAM: &str = r#"DELETE FROM "LiteLLM_TeamTable" WHERE "team_id" = $1"#;
const INSERT_USER: &str =
    r#"INSERT INTO "LiteLLM_UserTable" ("user_id", "user_email", "teams") VALUES ($1, $2, $3)"#;
const INSERT_MEMBERSHIP: &str =
    r#"INSERT INTO "LiteLLM_TeamMembership" ("user_id", "team_id") VALUES ($1, $2)"#;
const INSERT_KEY: &str = r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "user_id", "team_id", "key_name") VALUES ($1, $2, $3, $1)"#;
const DELETE_KEY_DIRECTLY: &str = r#"DELETE FROM "LiteLLM_VerificationToken" WHERE "token" = $1"#;
const SELECT_ROSTER: &str =
    r#"SELECT ("members_with_roles" #>> '{}') FROM "LiteLLM_TeamTable" WHERE "team_id" = $1"#;
const SELECT_TEAMS: &str =
    r#"SELECT array_to_json("teams")::TEXT FROM "LiteLLM_UserTable" WHERE "user_id" = $1"#;
const SELECT_KEY_COUNT: &str = r#"SELECT COUNT(*)::BIGINT FROM "LiteLLM_VerificationToken""#;
const SELECT_ARCHIVED_COUNT: &str =
    r#"SELECT COUNT(*)::BIGINT FROM "LiteLLM_DeletedVerificationToken""#;
const SELECT_TEAM_EXISTS: &str =
    r#"SELECT EXISTS (SELECT 1 FROM "LiteLLM_TeamTable" WHERE "team_id" = $1)"#;
const SELECT_MEMBERSHIP: &str = r#"SELECT EXISTS (SELECT 1 FROM "LiteLLM_TeamMembership" WHERE "team_id" = $1 AND "user_id" = $2)"#;
const INSERT_FAILURE: &str = r#"INSERT INTO test_failures ("stage") VALUES ($1)"#;
const CLEAR_FAILURES: &str = "DELETE FROM test_failures";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Harness {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
}

impl Harness {
    async fn open() -> Self {
        let url = Environment::parse_from(["bulk-team-removal-tests"])
            .url
            .map(SecretString::from);
        let mut task = PostgresFixture::provision_with_statements(
            url.as_ref(),
            vec![
                BASELINE.to_owned(),
                DELETED_TOKENS.to_owned(),
                TRIGGERS.to_owned(),
            ],
        );
        let fixture = task.resolve().await.unwrap();
        let config: Config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(6)
            .build()
            .unwrap();
        let store = PostgresStore {
            pool: Some(pool.clone()),
        };
        Self {
            fixture,
            pool,
            store,
        }
    }

    async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }

    async fn exec(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.unwrap();
        client.execute(sql, params).await.unwrap();
    }

    async fn count(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> i64 {
        let client = self.pool.get().await.unwrap();
        client.query_one(sql, params).await.unwrap().get(0)
    }

    async fn flag(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> bool {
        let client = self.pool.get().await.unwrap();
        client.query_one(sql, params).await.unwrap().get(0)
    }

    async fn json(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> JsonValue {
        let client = self.pool.get().await.unwrap();
        let text: String = client.query_one(sql, params).await.unwrap().get(0);
        serde_json::from_str(&text).unwrap()
    }

    async fn seed_team(&self, team_id: &str, roster: &str) {
        self.exec(INSERT_TEAM, &[&team_id, &roster]).await;
    }

    async fn seed_user(&self, user_id: &str, user_email: Option<&str>, teams: &[&str]) {
        let teams: Vec<String> = teams.iter().map(|team| (*team).to_owned()).collect();
        let params: [&(dyn ToSql + Sync); 3] = [&user_id, &user_email, &teams];
        self.exec(INSERT_USER, &params).await;
    }

    async fn seed_membership(&self, team_id: &str, user_id: &str) {
        self.exec(INSERT_MEMBERSHIP, &[&user_id, &team_id]).await;
    }

    async fn seed_key(&self, token: &str, team_id: &str, user_id: &str) {
        self.exec(INSERT_KEY, &[&token, &user_id, &team_id]).await;
    }

    async fn fail(&self, stage: &str) {
        self.exec(INSERT_FAILURE, &[&stage]).await;
    }

    async fn clear_failures(&self) {
        self.exec(CLEAR_FAILURES, &[]).await;
    }

    async fn roster(&self, team_id: &str) -> Vec<String> {
        let value = self.json(SELECT_ROSTER, &[&team_id]).await;
        roster_identities(&value)
    }

    async fn user_teams(&self, user_id: &str) -> Vec<String> {
        let value = self.json(SELECT_TEAMS, &[&user_id]).await;
        array_strings(&value)
    }

    async fn key_count(&self) -> i64 {
        self.count(SELECT_KEY_COUNT, &[]).await
    }

    async fn archived_count(&self) -> i64 {
        self.count(SELECT_ARCHIVED_COUNT, &[]).await
    }

    async fn team_exists(&self, team_id: &str) -> bool {
        self.flag(SELECT_TEAM_EXISTS, &[&team_id]).await
    }

    async fn has_membership(&self, team_id: &str, user_id: &str) -> bool {
        self.flag(SELECT_MEMBERSHIP, &[&team_id, &user_id]).await
    }

    async fn seed_committed_removal(&self) {
        self.seed_team("team-1", ROSTER_U1_U2).await;
        self.seed_user("u1", None, &["team-1"]).await;
        self.seed_membership("team-1", "u1").await;
        self.seed_key("k1", "team-1", "u1").await;
    }

    async fn assert_committed(&self) {
        assert_eq!(self.roster("team-1").await, support::lines(&["u2"]));
        assert_eq!(self.user_teams("u1").await, Vec::<String>::new());
        assert!(!self.has_membership("team-1", "u1").await);
        assert_eq!(self.key_count().await, 0);
        assert_eq!(self.archived_count().await, 1);
    }
}

fn roster_identities(value: &JsonValue) -> Vec<String> {
    let entries = value.as_array().cloned().unwrap_or_default();
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("user_id")
                .and_then(JsonValue::as_str)
                .or_else(|| entry.get("user_email").and_then(JsonValue::as_str))
                .map(str::to_owned)
        })
        .collect()
}

fn array_strings(value: &JsonValue) -> Vec<String> {
    let entries = value.as_array().cloned().unwrap_or_default();
    entries
        .iter()
        .filter_map(|entry| entry.as_str().map(str::to_owned))
        .collect()
}

async fn run(
    fixture: &support::Fixture,
    session: &dyn TeamRemovalSession,
    team_id: &str,
    members: &[MemberIdentity],
) -> Result<Vec<TeamMemberDeleteResult>, BulkRemovalError> {
    fixture
        .service(session)
        .bulk_remove_team_members(team_id, members, &support::proxy_admin())
        .await
}

struct Vanishing<'harness> {
    store: &'harness PostgresStore,
    clock: &'harness dyn ArchiveClock,
    pool: &'harness Pool,
    timeout: Duration,
}

impl Vanishing<'_> {
    fn removal(&self) -> PostgresTeamRemoval<'_> {
        PostgresTeamRemoval {
            store: self.store,
            clock: self.clock,
            timeout: self.timeout,
        }
    }

    async fn exec(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, tokio_postgres::Error> {
        let client = self.pool.get().await.unwrap();
        client.execute(sql, params).await
    }
}

#[async_trait]
impl TeamRemovalSession for Vanishing<'_> {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, BulkRemovalError> {
        let found = self.removal().find_team(team_id).await?;
        self.exec(DELETE_TEAM, &[&team_id]).await.unwrap();
        Ok(found)
    }

    async fn remove_members(
        &self,
        team_id: &str,
        members: &[MemberIdentity],
        caller: &CallerIdentity,
        jwt: &dyn JwtMappingSource,
    ) -> Result<TeamRemoval, BulkRemovalError> {
        self.removal()
            .remove_members(team_id, members, caller, jwt)
            .await
    }
}

fn assert_stage_error(stage: &str, error: &BulkRemovalError) {
    if stage == "archive" {
        assert!(
            matches!(error, BulkRemovalError::Archive(_)),
            "{stage}: {error:?}"
        );
    } else {
        assert!(
            matches!(error, BulkRemovalError::Database(_)),
            "{stage}: {error:?}"
        );
    }
}

pub(super) async fn concurrent_roster_delete() {
    lock_before_fresh_read().await;
    two_removals_keep_each_others_members().await;
}

async fn lock_before_fresh_read() {
    let harness = Harness::open().await;
    harness.seed_team("team-1", ROSTER_U1_U2).await;
    harness.seed_user("u1", None, &["team-1"]).await;
    harness.seed_user("u2", None, &["team-1"]).await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let holder = harness.pool.get().await.unwrap();
    holder.batch_execute("BEGIN").await.unwrap();
    holder
        .query(TEAM_ADVISORY_LOCK_SQL, &[&"team-1"])
        .await
        .unwrap();
    holder
        .execute(SET_ROSTER, &[&"team-1", &ROSTER_U1_U2_U3])
        .await
        .unwrap();
    let pending_members = [support::identity_id("u1")];
    let mut pending = Box::pin(run(&fixture, &removal, "team-1", &pending_members));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut pending)
            .await
            .is_err()
    );
    holder.batch_execute("COMMIT").await.unwrap();
    drop(holder);
    let results = pending.await.unwrap();
    assert!(results[0].success);
    assert_eq!(
        harness.roster("team-1").await,
        support::lines(&["u2", "u3"])
    );
    harness.close().await;
}

async fn two_removals_keep_each_others_members() {
    let harness = Harness::open().await;
    harness.seed_team("team-1", ROSTER_U1_U2_U3).await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let first_members = [support::identity_id("u1")];
    let second_members = [support::identity_id("u2")];
    let first = run(&fixture, &removal, "team-1", &first_members);
    let second = run(&fixture, &removal, "team-1", &second_members);
    let (first, second) = tokio::join!(first, second);
    assert!(first.unwrap()[0].success);
    assert!(second.unwrap()[0].success);
    assert_eq!(harness.roster("team-1").await, support::lines(&["u3"]));
    harness.close().await;
}

pub(super) async fn delete_rollback() {
    assert_eq!(BATCH_TX_TIMEOUT, Duration::from_secs(60));
    timeout_rolls_back().await;
    write_failures_roll_back().await;
    archive_precedes_delete().await;
}

async fn timeout_rolls_back() {
    let harness = Harness::open().await;
    harness.seed_committed_removal().await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: Duration::from_millis(100),
    };
    let holder = harness.pool.get().await.unwrap();
    holder.batch_execute("BEGIN").await.unwrap();
    holder
        .query(TEAM_ADVISORY_LOCK_SQL, &[&"team-1"])
        .await
        .unwrap();
    let error = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap_err();
    assert!(matches!(error, BulkRemovalError::Timeout { seconds: 0 }));
    holder.batch_execute("ROLLBACK").await.unwrap();
    drop(holder);
    assert_eq!(
        harness.roster("team-1").await,
        support::lines(&["u1", "u2"])
    );
    assert_eq!(harness.key_count().await, 1);
    assert_eq!(harness.archived_count().await, 0);
    assert!(harness.has_membership("team-1", "u1").await);
    harness.close().await;
}

async fn write_failures_roll_back() {
    for stage in [
        "roster",
        "user-teams",
        "membership",
        "archive",
        "key-delete",
    ] {
        let harness = Harness::open().await;
        harness.seed_committed_removal().await;
        harness.fail(stage).await;
        let fixture = support::fixture();
        let clock = SystemArchiveClock;
        let removal = PostgresTeamRemoval {
            store: &harness.store,
            clock: &clock,
            timeout: BATCH_TX_TIMEOUT,
        };
        let error = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
            .await
            .unwrap_err();
        assert_stage_error(stage, &error);
        assert_eq!(
            harness.roster("team-1").await,
            support::lines(&["u1", "u2"])
        );
        assert_eq!(harness.user_teams("u1").await, support::lines(&["team-1"]));
        assert!(harness.has_membership("team-1", "u1").await);
        assert_eq!(harness.key_count().await, 1);
        assert_eq!(harness.archived_count().await, 0);
        harness.clear_failures().await;
        let results = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
            .await
            .unwrap();
        assert!(results[0].success);
        harness.assert_committed().await;
        harness.close().await;
    }
}

async fn archive_precedes_delete() {
    let harness = Harness::open().await;
    harness.seed_committed_removal().await;
    let early = {
        let client = harness.pool.get().await.unwrap();
        client.execute(DELETE_KEY_DIRECTLY, &[&"k1"]).await
    };
    assert!(early.is_err());
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let results = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap();
    assert!(results[0].success);
    assert_eq!(harness.key_count().await, 0);
    assert_eq!(harness.archived_count().await, 1);
    harness.close().await;
}

pub(super) async fn post_commit_effects() {
    post_commit_order().await;
    vanished_team().await;
    stale_array_only_membership().await;
    email_only_roster().await;
    no_key_path().await;
    post_commit_failures().await;
}

async fn post_commit_order() {
    let harness = Harness::open().await;
    harness.seed_committed_removal().await;
    let fixture = support::fixture();
    fixture.jwt.add_mapping("k1", "sub", "claim-k1", "iss");
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let results = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap();
    assert!(results[0].success);
    let events = fixture.log.events();
    support::assert_step_order(
        &events,
        &["audit:", "key-cache:", "publish:", "mapping:", "metric:"],
    );
    assert!(
        events
            .iter()
            .any(|event| event == "mapping:jwt_key_mapping:iss:sub:claim-k1")
    );
    harness.assert_committed().await;
    harness.close().await;
}

async fn vanished_team() {
    let harness = Harness::open().await;
    harness.seed_team("team-1", ROSTER_U1).await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let session = Vanishing {
        store: &harness.store,
        clock: &clock,
        pool: &harness.pool,
        timeout: BATCH_TX_TIMEOUT,
    };
    let error = run(&fixture, &session, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap_err();
    let BulkRemovalError::Problem(problem) = error else {
        panic!("a vanished team is a problem");
    };
    assert_eq!(problem.0.status, 404);
    assert_eq!(problem.0.r#type, "urn:litellm:error:team-not-found");
    assert_eq!(problem.0.detail, "Team id=team-1 does not exist in db");
    assert!(!harness.team_exists("team-1").await);
    harness.close().await;
}

async fn stale_array_only_membership() {
    let harness = Harness::open().await;
    harness.seed_team("team-1", ROSTER_U2).await;
    harness
        .seed_user("u1", None, &["team-1", "other", "team-1"])
        .await;
    harness.seed_membership("team-1", "u1").await;
    harness.seed_key("k1", "team-1", "u1").await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let results = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap();
    assert!(results[0].success);
    assert_eq!(harness.roster("team-1").await, support::lines(&["u2"]));
    assert_eq!(harness.user_teams("u1").await, support::lines(&["other"]));
    assert!(!harness.has_membership("team-1", "u1").await);
    assert_eq!(harness.key_count().await, 0);
    assert_eq!(harness.archived_count().await, 1);
    harness.close().await;
}

async fn email_only_roster() {
    let harness = Harness::open().await;
    harness.seed_team("team-1", EMAIL_ROSTER).await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let members = [support::identity(None, Some("a@example.com"))];
    let results = run(&fixture, &removal, "team-1", &members).await.unwrap();
    assert!(results[0].success);
    assert_eq!(harness.roster("team-1").await, Vec::<String>::new());
    harness.close().await;
}

async fn no_key_path() {
    let harness = Harness::open().await;
    harness.seed_team("team-1", ROSTER_U1).await;
    let fixture = support::fixture();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let results = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap();
    assert!(results[0].success);
    assert_eq!(harness.roster("team-1").await, Vec::<String>::new());
    assert_eq!(harness.key_count().await, 0);
    assert_eq!(harness.archived_count().await, 0);
    let events = fixture.log.events();
    assert!(
        events
            .iter()
            .all(|event| !event.starts_with("audit:") && !event.starts_with("key-cache:"))
    );
    assert!(events.iter().any(|event| event.starts_with("metric:")));
    harness.close().await;
}

async fn post_commit_failures() {
    audit_failure_stops_later_steps().await;
    cache_failure_is_swallowed().await;
    mapping_failure_is_swallowed().await;
    metric_failure_keeps_the_commit().await;
}

async fn audit_failure_stops_later_steps() {
    let harness = Harness::open().await;
    harness.seed_committed_removal().await;
    let fixture = support::fixture();
    fixture.audit.failing.store(true, Relaxed);
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let error = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap_err();
    assert!(matches!(error, BulkRemovalError::MissingRow));
    harness.assert_committed().await;
    let events = fixture.log.events();
    assert!(
        events
            .iter()
            .any(|event| event.starts_with("audit-failed:"))
    );
    assert!(events.iter().all(|event| {
        !event.starts_with("key-cache:")
            && !event.starts_with("mapping:")
            && !event.starts_with("metric:")
    }));
    harness.close().await;
}

#[derive(Clone, Copy)]
enum Failure {
    Cache,
    Mapping,
}

async fn failure_case(failure: Failure) -> (support::Fixture, Harness) {
    let harness = Harness::open().await;
    harness.seed_committed_removal().await;
    let fixture = support::fixture();
    match failure {
        Failure::Cache => fixture.auth.fail_delete_sync.store(true, Relaxed),
        Failure::Mapping => fixture.auth.fail_delete_memory.store(true, Relaxed),
    }
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let results = run(&fixture, &removal, "team-1", &[support::identity_id("u1")])
        .await
        .unwrap();
    assert!(results[0].success);
    harness.assert_committed().await;
    (fixture, harness)
}

async fn cache_failure_is_swallowed() {
    let (fixture, harness) = failure_case(Failure::Cache).await;
    let events = fixture.log.events();
    assert!(events.iter().any(|event| event.starts_with("mapping:")));
    assert!(events.iter().any(|event| event.starts_with("metric:")));
    harness.close().await;
}

async fn mapping_failure_is_swallowed() {
    let (fixture, harness) = failure_case(Failure::Mapping).await;
    let events = fixture.log.events();
    assert!(
        events
            .iter()
            .any(|event| event.starts_with("mapping-failed:"))
    );
    assert!(events.iter().any(|event| event.starts_with("metric:")));
    harness.close().await;
}

async fn metric_failure_keeps_the_commit() {
    let harness = Harness::open().await;
    harness.seed_committed_removal().await;
    let fixture = support::fixture();
    let context = support::mismatched_context(&fixture.settings);
    let access = fixture.team_access();
    let clock = SystemArchiveClock;
    let removal = PostgresTeamRemoval {
        store: &harness.store,
        clock: &clock,
        timeout: BATCH_TX_TIMEOUT,
    };
    let service = BulkTeamRemoval {
        session: &removal,
        access: &access,
        jwt: &fixture.jwt,
        audit: &fixture.audit,
        invalidation: &fixture.invalidation,
        gauge: Some(&fixture.gauge),
        labels: LabelSource::Context(&context),
    };
    let results = service
        .bulk_remove_team_members(
            "team-1",
            &[support::identity_id("u1")],
            &support::proxy_admin(),
        )
        .await
        .unwrap();
    assert!(results[0].success);
    harness.assert_committed().await;
    assert!(
        fixture
            .log
            .events()
            .iter()
            .all(|event| !event.starts_with("metric:"))
    );
    harness.close().await;
}
