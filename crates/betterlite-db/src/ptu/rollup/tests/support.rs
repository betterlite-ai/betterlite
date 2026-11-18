use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::Client;
use futures_util::future::BoxFuture;
use uuid::Uuid;

use rust_decimal::Decimal;

// The store's own test support, reused rather than copied: fixtures, log capture, a recording sleeper and stored-model rows.
#[path = "../../store/tests/support.rs"]
mod stored;

pub(crate) use stored::{
    Recorder, capture_logs, charge, close, database, insert_stored_model, names, write_for,
};

use crate::budget_permission_rows::JsonData;
use crate::database_failures::metrics::WallClock;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::deployment::RouterEntry;
use crate::ptu::charges::records::{BackfillResult, RollupResult};
use crate::ptu::rollup::RollupError;
use crate::ptu::rollup::alert::OperatorAlert;
use crate::ptu::rollup::backfill::{BackfillRun, run_ptu_flat_cost_backfill};
use crate::ptu::rollup::daily::{DailyRun, run_ptu_flat_cost_rollup};
use crate::ptu::rollup::scheduled::{
    PTU_ROLLUP_LOCK_TTL_SECONDS, ScheduledRun, run_scheduled_ptu_rollup,
};
use crate::queues::pod_lock::owner::PodId;
use crate::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};

pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl WallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test alert channel is down")]
pub(crate) struct AlertDown;

#[derive(Default)]
pub(crate) struct AlertLog {
    pub(crate) messages: Mutex<Vec<String>>,
    pub(crate) down: bool,
}

impl AlertLog {
    pub(crate) fn down() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            down: true,
        }
    }

    pub(crate) fn messages(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }
}

impl OperatorAlert for AlertLog {
    type Error = AlertDown;

    fn send<'message>(
        &'message self,
        message: &'message str,
    ) -> BoxFuture<'message, Result<(), AlertDown>> {
        Box::pin(async move {
            if self.down {
                return Err(AlertDown);
            }
            self.messages.lock().unwrap().push(message.to_owned());
            Ok(())
        })
    }
}

#[derive(Default)]
pub(crate) struct FakeLease {
    pub(crate) owner: Mutex<Option<String>>,
    pub(crate) read_fails: bool,
    pub(crate) acquire_fails: bool,
    pub(crate) deletes: Mutex<usize>,
    pub(crate) keys: Mutex<Vec<String>>,
    pub(crate) ttls: Mutex<Vec<i64>>,
}

impl FakeLease {
    pub(crate) fn held_by(owner: &str) -> Self {
        Self {
            owner: Mutex::new(Some(owner.to_owned())),
            read_fails: false,
            acquire_fails: false,
            deletes: Mutex::new(0),
            keys: Mutex::new(Vec::new()),
            ttls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn deletes(&self) -> usize {
        *self.deletes.lock().unwrap()
    }

    pub(crate) fn keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }

    pub(crate) fn ttls(&self) -> Vec<i64> {
        self.ttls.lock().unwrap().clone()
    }
}

impl LockCache for FakeLease {
    async fn get_owner(&self, _key: &str) -> Result<Option<Vec<u8>>, LockCacheError> {
        if self.read_fails {
            return Err(LockCacheError::UnexpectedReply {
                operation: "get_owner",
            });
        }
        Ok(self.owner.lock().unwrap().clone().map(String::into_bytes))
    }

    async fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> Result<bool, LockCacheError> {
        if self.acquire_fails {
            return Err(LockCacheError::UnexpectedReply {
                operation: "set_owner_if_absent",
            });
        }
        self.keys.lock().unwrap().push(key.to_owned());
        self.ttls.lock().unwrap().push(ttl_seconds);
        let mut held = self.owner.lock().unwrap();
        if held.is_some() {
            return Ok(false);
        }
        *held = Some(owner.to_owned());
        Ok(true)
    }

    async fn delete(&self, _key: &str) -> Result<i64, LockCacheError> {
        let removed = self.owner.lock().unwrap().take();
        let mut deletes = self.deletes.lock().unwrap();
        *deletes = deletes.saturating_add(1);
        Ok(i64::from(removed.is_some()))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

pub(crate) async fn ddl(client: &Client, sql: &str) {
    client.batch_execute(sql).await.unwrap();
}

pub(crate) fn manager(lease: &Arc<FakeLease>) -> PodLockManager<FakeLease> {
    PodLockManager::new(
        Some(Arc::clone(lease)),
        PTU_ROLLUP_LOCK_TTL_SECONDS,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(Uuid::new_v4()).unwrap(),
    )
}

// A lease whose write fails, with the later owner read either answering "absent" or failing too.
pub(crate) fn manager_that_cannot_acquire(read_fails: bool) -> PodLockManager<FakeLease> {
    let lease = Arc::new(FakeLease {
        owner: Mutex::new(None),
        read_fails,
        acquire_fails: true,
        deletes: Mutex::new(0),
        keys: Mutex::new(Vec::new()),
        ttls: Mutex::new(Vec::new()),
    });
    manager(&lease)
}

pub(crate) fn cacheless_manager() -> PodLockManager<FakeLease> {
    PodLockManager::new(
        None,
        PTU_ROLLUP_LOCK_TTL_SECONDS,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(Uuid::new_v4()).unwrap(),
    )
}

pub(crate) fn store(pool: &deadpool_postgres::Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

pub(crate) fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

pub(crate) fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

pub(crate) fn info(source: &str) -> JsonData {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn ptu_info(
    team: &str,
    count: u32,
    rate: &str,
    from: &str,
    to: Option<&str>,
) -> JsonData {
    let end = to.map_or_else(String::new, |end| {
        format!(r#", "ptu_effective_to": "{end}""#)
    });
    info(&format!(
        r#"{{"ptu_count": {count}, "cost_per_ptu_per_hour": {rate}, "team_id": "{team}", "ptu_effective_from": "{from}"{end}}}"#
    ))
}

pub(crate) fn entry(id: &str, model_name: &str, model_info: &JsonData) -> RouterEntry {
    let JsonData::Object(mut fields) = model_info.clone() else {
        panic!("a config deployment's model_info is an object");
    };
    fields.insert("id".to_owned(), JsonData::String(id.to_owned()));
    RouterEntry {
        model_name: model_name.to_owned(),
        model_info: JsonData::Object(fields),
    }
}

pub(crate) fn clock() -> FixedClock {
    FixedClock(instant("2026-03-01T12:00:00Z"))
}

// A deployment whose window covers every day these tests use: one unit per hour, so a full day costs 24.
pub(crate) fn active_info(team: &str) -> JsonData {
    ptu_info(team, 1, "1", "2026-01-01T00:00:00Z", None)
}

// A deployment whose window closed before the tests' clock, so it is scanned but accrues nothing.
pub(crate) fn ended_info(team: &str, to: &str) -> JsonData {
    ptu_info(team, 1, "1", "2026-01-01T00:00:00Z", Some(to))
}

pub(crate) fn no_database() -> PostgresStore {
    PostgresStore { pool: None }
}

pub(crate) async fn seed(client: &Client, rows: &str) {
    ddl(
        client,
        &format!(
            "INSERT INTO \"LiteLLM_DailyTeamSpend\" \
             (\"id\", \"team_id\", \"date\", \"api_key\", \"model\", \"model_group\", \
              \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\", \"spend\", \
              \"api_requests\", \"prompt_tokens\", \"created_at\", \"updated_at\", \"ptu_flat_cost\") \
             VALUES {rows}"
        ),
    )
    .await;
}

pub(crate) async fn seed_sentinel(client: &Client, date: &str, model: &str, cost: &str) {
    let rows = format!(
        "('{model}', 'team-1', '{date}', '__ptu_flat_cost__', '{model}', '{model}', \
          '', '', '', 0, 0, 0, now(), '{date} 11:00:00', {cost})"
    );
    seed(client, &rows).await;
}

// A deployment whose window closed before the run, so the day scans it and writes no charge for it.
pub(crate) async fn seed_lapsed_deployment(client: &Client, date: &str) {
    let info = ended_info("team-1", "2026-02-15T00:00:00Z");
    insert_stored_model(client, "model-z", "name-z", &info).await;
    seed_sentinel(client, date, "model-z", "3.5").await;
}

// More lapsed deployments than the alert's name limit, each scanned but accruing nothing.
pub(crate) async fn seed_lapsed_fleet(client: &Client, count: u32) {
    for index in 0..count {
        let name = format!("lapsed-{index}");
        let info = ended_info("team-1", "2026-02-15T00:00:00Z");
        insert_stored_model(client, &name, &name, &info).await;
    }
}

fn rejection_sql(column: &str, value: &str) -> String {
    format!(
        "CREATE FUNCTION ptu_reject_write() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.{column} = '{value}' THEN \
             RAISE EXCEPTION 'the charge cannot be written'; \
           END IF; \
           RETURN NEW; \
         END; \
         $$; \
         CREATE TRIGGER ptu_reject_write_trigger BEFORE INSERT OR UPDATE \
         ON \"LiteLLM_DailyTeamSpend\" FOR EACH ROW EXECUTE FUNCTION ptu_reject_write();"
    )
}

// Fails every write whose row matches this column value, so each retrying upsert for that charge exhausts its attempts.
pub(crate) async fn reject_writes(client: &Client, column: &str, value: &str) {
    let sql = rejection_sql(column, value);
    ddl(client, &sql).await;
}

pub(crate) async fn logged_writes(client: &Client) -> Vec<String> {
    let rows = client
        .query("SELECT model, date FROM ptu_write_log ORDER BY seq", &[])
        .await
        .unwrap();
    rows.iter()
        .map(|row| {
            format!(
                "{}|{}",
                row.get::<_, String>("model"),
                row.get::<_, String>("date")
            )
        })
        .collect()
}

pub(crate) async fn cell(client: &Client, column: &str, date: &str, model: &str) -> Option<String> {
    let sql = format!(
        "SELECT \"{column}\"::text FROM \"LiteLLM_DailyTeamSpend\" \
         WHERE \"date\" = $1 AND \"model\" = $2"
    );
    let row = client.query_opt(&sql, &[&date, &model]).await.unwrap()?;
    Some(row.get(0))
}

pub(crate) async fn cost_of(client: &Client, date: &str, model: &str) -> Option<Decimal> {
    cell(client, "ptu_flat_cost", date, model)
        .await
        .map(|text| text.parse().unwrap())
}

pub(crate) async fn models_on(client: &Client, date: &str) -> Vec<String> {
    let rows = client
        .query(
            "SELECT \"model\" FROM \"LiteLLM_DailyTeamSpend\" WHERE \"date\" = $1 ORDER BY \"model\"",
            &[&date],
        )
        .await
        .unwrap();
    rows.iter().map(|row| row.get("model")).collect()
}

pub(crate) async fn row_count(client: &Client) -> i64 {
    client
        .query_one("SELECT COUNT(*) FROM \"LiteLLM_DailyTeamSpend\"", &[])
        .await
        .unwrap()
        .get(0)
}

pub(crate) async fn daily(
    store: &PostgresStore,
    entries: &[RouterEntry],
    clock: &FixedClock,
    target: Option<NaiveDate>,
    may_prune: bool,
) -> Result<RollupResult, RollupError> {
    let mut sleeper = Recorder::default();
    run_ptu_flat_cost_rollup(DailyRun {
        store,
        entries,
        clock,
        sleeper: &mut sleeper,
        target_date: target,
        may_prune,
    })
    .await
}

pub(crate) async fn backfill(
    store: &PostgresStore,
    clock: &FixedClock,
    today: Option<NaiveDate>,
) -> Result<BackfillResult, RollupError> {
    let mut sleeper = Recorder::default();
    run_ptu_flat_cost_backfill(BackfillRun {
        store,
        entries: &[],
        clock,
        sleeper: &mut sleeper,
        today,
    })
    .await
}

pub(crate) struct Schedule<'run> {
    pub(crate) store: &'run PostgresStore,
    pub(crate) clock: &'run FixedClock,
    pub(crate) lock: Option<&'run PodLockManager<FakeLease>>,
    pub(crate) alert: Option<&'run AlertLog>,
    pub(crate) attribution_enabled: bool,
    pub(crate) target_date: Option<NaiveDate>,
}

pub(crate) async fn scheduled(run: Schedule<'_>) -> Result<Option<RollupResult>, RollupError> {
    let mut sleeper = Recorder::default();
    run_scheduled_ptu_rollup(ScheduledRun {
        store: run.store,
        entries: &[],
        clock: run.clock,
        sleeper: &mut sleeper,
        lock: run.lock,
        alert: run.alert,
        attribution_enabled: run.attribution_enabled,
        target_date: run.target_date,
    })
    .await
}
