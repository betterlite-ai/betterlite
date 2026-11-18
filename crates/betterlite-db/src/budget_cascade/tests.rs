#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;

use super::BudgetCascade;
use super::linked_rows::{
    KeyRow, LinkedRows, ModelAccessGroupRow, OrganizationRow, ProjectRow, TagRow, TeamMembershipRow,
};
use super::rollover_order::LinkedResets;
use crate::budget_permission_rows::{LiteLLMBudgetTable, LiteLLMBudgetTableFull, Timestamp};
use crate::budget_reset_policy::cache_invalidation::{CacheDeleteFuture, ResetCache};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::postgres_store::session::PostgresStore;
use crate::spend_reset_unit_of_work::queue::LinkedResetFilter;

const CASCADE_TABLES: &str = "CREATE TABLE \"LiteLLM_BudgetTable\" (budget_id TEXT PRIMARY KEY, budget_reset_at TIMESTAMP(3)); CREATE TABLE \"LiteLLM_TeamMembership\" (user_id TEXT, team_id TEXT, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0, PRIMARY KEY (user_id, team_id)); CREATE TABLE \"LiteLLM_VerificationToken\" (token TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0, budget_duration TEXT); CREATE TABLE \"LiteLLM_OrganizationTable\" (organization_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_TagTable\" (tag_name TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_ModelAccessGroupBudgetTable\" (access_group_name TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_ProjectTable\" (project_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_EndUserTable\" (user_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0);";

const SEED_LINKED_ROWS: &str = "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id) VALUES ('budget-a'); INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('user-a', 'team-a', 'budget-a', 10); INSERT INTO \"LiteLLM_VerificationToken\" (token, budget_id, spend) VALUES ('token-a', 'budget-a', 10), ('token-window', 'budget-a', 10); UPDATE \"LiteLLM_VerificationToken\" SET budget_duration = 'daily' WHERE token = 'token-window'; INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, budget_id, spend) VALUES ('org-a', 'budget-a', 10); INSERT INTO \"LiteLLM_TagTable\" (tag_name, budget_id, spend) VALUES ('tag-a', 'budget-a', 10); INSERT INTO \"LiteLLM_ModelAccessGroupBudgetTable\" (access_group_name, budget_id, spend) VALUES ('group-a', 'budget-a', 10); INSERT INTO \"LiteLLM_ProjectTable\" (project_id, budget_id, spend) VALUES ('project-a', 'budget-a', 10); INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('customer-linked', 'budget-a', 10), ('customer-null', NULL, 10);";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().expect("fixture url parses");
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager)
        .max_size(2)
        .build()
        .expect("pool builds")
}

async fn provision_fixture() -> PostgresFixture {
    let url = Environment::parse_from(["budget-cascade-tests"])
        .url
        .map(SecretString::from);
    PostgresFixture::provision_with_statements(url.as_ref(), vec![CASCADE_TABLES.to_owned()])
        .resolve()
        .await
        .expect("fixture provisions")
}

pub(crate) struct Database {
    pub(crate) fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    pub(crate) client: Client,
}

impl Database {
    pub(crate) async fn provision() -> Self {
        let fixture = provision_fixture().await;
        let store = PostgresStore {
            pool: Some(pool(&fixture.options.url)),
        };
        let client = store.session().await.expect("fixture session");
        Self {
            fixture,
            store,
            client,
        }
    }

    pub(crate) async fn finish(self) {
        let Self {
            fixture,
            store,
            client,
        } = self;
        drop(client);
        store.pool.expect("fixture store holds a pool").close();
        fixture.close_async().await.expect("fixture closes");
    }
}

pub(crate) async fn seed_linked_rows(client: &Client) {
    client
        .batch_execute(SEED_LINKED_ROWS)
        .await
        .expect("seed rows");
}

pub(crate) async fn spend_of(client: &Client, table: &str, column: &str, id: &str) -> Decimal {
    let sql = format!("SELECT spend::numeric FROM \"{table}\" WHERE {column} = $1");
    let row = client
        .query_one(sql.as_str(), &[&id])
        .await
        .expect("row exists");
    row.get(0)
}

pub(crate) async fn reset_at_of(client: &Client, budget_id: &str) -> Option<NaiveDateTime> {
    client
        .query_one(
            "SELECT budget_reset_at FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
            &[&budget_id],
        )
        .await
        .expect("budget row exists")
        .get(0)
}

pub(crate) async fn reject_budget_updates(client: &Client) {
    client
        .batch_execute(
            "CREATE FUNCTION reject_budget_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'budget table is locked'; END $$; CREATE TRIGGER reject_budget_update BEFORE UPDATE ON \"LiteLLM_BudgetTable\" FOR EACH ROW EXECUTE FUNCTION reject_budget_update();",
        )
        .await
        .expect("trigger installs");
}

pub(crate) fn budget(
    budget_id: &str,
    max_budget: Option<Decimal>,
    duration: Option<&str>,
) -> LiteLLMBudgetTableFull {
    LiteLLMBudgetTableFull {
        budget: LiteLLMBudgetTable {
            budget_id: Some(budget_id.to_owned()),
            soft_budget: None,
            max_budget,
            max_parallel_requests: None,
            tpm_limit: None,
            rpm_limit: None,
            tpd_limit: None,
            model_max_budget: None,
            budget_duration: duration.map(str::to_owned),
            allowed_models: None,
            temp_budget_increase: None,
            temp_budget_expiry: None,
        },
        budget_reset_at: None,
        created_at: Timestamp::Naive(
            DateTime::<Utc>::from_timestamp(0, 0)
                .expect("epoch is valid")
                .naive_utc(),
        ),
    }
}

pub(crate) fn unnamed_budget() -> LiteLLMBudgetTableFull {
    let mut full = budget("unnamed", Some(Decimal::from(4_i32)), Some("daily"));
    full.budget.budget_id = None;
    full
}

pub(crate) fn cascade(
    budget_ids: &[&str],
    rollover_caps: BTreeMap<String, Decimal>,
) -> BudgetCascade {
    BudgetCascade {
        budgets: Vec::new(),
        budget_ids: budget_ids
            .iter()
            .map(|budget_id| (*budget_id).to_owned())
            .collect(),
        budget_resets: Vec::new(),
        counter_resets: Vec::new(),
        cache_keys: Vec::new(),
        rollover_caps,
    }
}

pub(crate) struct RecordingSchedule {
    pub(crate) at: DateTime<Utc>,
    durations: Mutex<Vec<String>>,
}

impl RecordingSchedule {
    pub(crate) const fn new(at: DateTime<Utc>) -> Self {
        Self {
            at,
            durations: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn durations(&self) -> Vec<String> {
        self.durations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResetSchedule for RecordingSchedule {
    type Failure = Infallible;

    fn next_reset_at(&self, budget_duration: &str) -> Result<DateTime<Utc>, Infallible> {
        self.durations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(budget_duration.to_owned());
        Ok(self.at)
    }
}

pub(crate) struct FakeRows {
    pub(crate) team_memberships: Result<Vec<TeamMembershipRow>, String>,
    pub(crate) keys: Result<Vec<KeyRow>, String>,
    pub(crate) organizations: Result<Vec<OrganizationRow>, String>,
    pub(crate) tags: Result<Vec<TagRow>, String>,
    pub(crate) model_access_groups: Result<Vec<ModelAccessGroupRow>, String>,
    pub(crate) projects: Result<Vec<ProjectRow>, String>,
}

impl FakeRows {
    pub(crate) const fn empty() -> Self {
        Self {
            team_memberships: Ok(Vec::new()),
            keys: Ok(Vec::new()),
            organizations: Ok(Vec::new()),
            tags: Ok(Vec::new()),
            model_access_groups: Ok(Vec::new()),
            projects: Ok(Vec::new()),
        }
    }
}

impl LinkedRows for FakeRows {
    type Failure = String;

    async fn team_memberships(
        &self,
        _budget_ids: &[String],
    ) -> Result<Vec<TeamMembershipRow>, String> {
        self.team_memberships.clone()
    }

    async fn keys(&self, _budget_ids: &[String]) -> Result<Vec<KeyRow>, String> {
        self.keys.clone()
    }

    async fn organizations(&self, _budget_ids: &[String]) -> Result<Vec<OrganizationRow>, String> {
        self.organizations.clone()
    }

    async fn tags(&self, _budget_ids: &[String]) -> Result<Vec<TagRow>, String> {
        self.tags.clone()
    }

    async fn model_access_groups(
        &self,
        _budget_ids: &[String],
    ) -> Result<Vec<ModelAccessGroupRow>, String> {
        self.model_access_groups.clone()
    }

    async fn projects(&self, _budget_ids: &[String]) -> Result<Vec<ProjectRow>, String> {
        self.projects.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedWrite {
    Zero(LinkedResetFilter),
    Decrement(LinkedResetFilter, Decimal),
}

#[derive(Debug, Default)]
pub(crate) struct RecordingWrites(Mutex<Vec<RecordedWrite>>);

impl RecordingWrites {
    pub(crate) fn writes(&self) -> Vec<RecordedWrite> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl LinkedResets for RecordingWrites {
    fn zero(&self, filter: LinkedResetFilter) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedWrite::Zero(filter));
    }

    fn decrement(&self, filter: LinkedResetFilter, amount: Decimal) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedWrite::Decrement(filter, amount));
    }
}

pub(crate) struct RecordingCache {
    pub(crate) probe: Option<PostgresStore>,
    counters: Mutex<Vec<Vec<String>>>,
    keys: Mutex<Vec<Vec<String>>>,
    observed_spend: Mutex<Vec<Decimal>>,
}

impl RecordingCache {
    const fn with_probe(probe: Option<PostgresStore>) -> Self {
        Self {
            probe,
            counters: Mutex::new(Vec::new()),
            keys: Mutex::new(Vec::new()),
            observed_spend: Mutex::new(Vec::new()),
        }
    }

    pub(crate) const fn probing(probe: PostgresStore) -> Self {
        Self::with_probe(Some(probe))
    }

    pub(crate) const fn silent() -> Self {
        Self::with_probe(None)
    }

    pub(crate) fn counters(&self) -> Vec<Vec<String>> {
        self.counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn keys(&self) -> Vec<Vec<String>> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn observed_spend(&self) -> Vec<Decimal> {
        self.observed_spend
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResetCache for RecordingCache {
    type Failure = Infallible;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, Infallible> {
        Box::pin(async move {
            self.counters
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(keys.to_vec());
            if let Some(store) = &self.probe {
                let client = store.session().await.expect("probe session");
                let row = client
                    .query_one(
                        "SELECT spend::numeric FROM \"LiteLLM_OrganizationTable\" WHERE organization_id = 'org-a'",
                        &[],
                    )
                    .await
                    .expect("organization row");
                let spend: Decimal = row.get(0);
                self.observed_spend
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(spend);
            }
            Ok(())
        })
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, Infallible> {
        Box::pin(async move {
            self.keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(keys.to_vec());
            Ok(())
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured logs are utf-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn subscriber(logs: Logs) -> impl Subscriber + Send + Sync + 'static {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(logs)
        .finish()
}
