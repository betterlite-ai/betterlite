#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_db::postgres_store::session::PostgresStore;
use tokio::time::Instant;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_LEGACY, ALPHA_STABLE, SCHEMA, TestDatabase, insert_row, loaded, row_agents,
};

const SKIPPING_TRIGGER: &str = r#"
CREATE FUNCTION skip_agent_rows() RETURNS trigger AS $$
BEGIN
    RETURN NULL;
END
$$ LANGUAGE plpgsql;
CREATE TRIGGER skip_agent_rows BEFORE UPDATE ON "LiteLLM_ObjectPermissionTable"
FOR EACH ROW EXECUTE FUNCTION skip_agent_rows();
"#;

const SKIP_FIRST_TRIGGER: &str = r#"
CREATE TABLE skip_first_state (object_permission_id TEXT PRIMARY KEY);
CREATE FUNCTION skip_first_update() RETURNS trigger AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM skip_first_state WHERE object_permission_id = OLD.object_permission_id) THEN
        RETURN NEW;
    END IF;
    INSERT INTO skip_first_state (object_permission_id) VALUES (OLD.object_permission_id);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;
CREATE TRIGGER skip_first_update BEFORE UPDATE ON "LiteLLM_ObjectPermissionTable"
FOR EACH ROW EXECUTE FUNCTION skip_first_update();
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Logged {
    level: Level,
    message: String,
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Logged>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let logged = Logged {
            level: *event.metadata().level(),
            message: message.0,
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(logged);
    }
}

fn capture() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

fn messages(logs: &Logs) -> Vec<Logged> {
    logs.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[tokio::test(start_paused = true)]
async fn a_clean_first_pass_stops_without_a_pause() {
    let db = TestDatabase::start(SCHEMA).await;
    insert_row(&db.pool, "p1", &[ALPHA_LEGACY.to_owned()]).await;
    let registry = loaded(&[ALPHA_ENTRY]);
    let (logs, _guard) = capture();
    let started = Instant::now();
    registry.run_grant_id_migration(&db.store()).await;
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(
        messages(&logs),
        vec![Logged {
            level: Level::INFO,
            message: String::from("Rewrote 1 object_permission rows from legacy config agent ids"),
        }]
    );
    assert_eq!(
        row_agents(&db.pool, "p1").await,
        Some(vec![ALPHA_STABLE.to_owned()])
    );
    db.close().await;
}

#[tokio::test(start_paused = true)]
async fn missed_rows_are_warned_three_times_with_a_pause_each() {
    let schema = format!("{SCHEMA}{SKIPPING_TRIGGER}");
    let db = TestDatabase::start(&schema).await;
    insert_row(&db.pool, "p1", &[ALPHA_LEGACY.to_owned()]).await;
    let registry = loaded(&[ALPHA_ENTRY]);
    let (logs, _guard) = capture();
    let started = Instant::now();
    registry.run_grant_id_migration(&db.store()).await;
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    let logged = messages(&logs);
    assert_eq!(logged.len(), 3);
    assert!(logged.iter().zip(1..=3).all(|(entry, attempt)| {
        entry.level == Level::WARN
            && entry.message
                == format!(
                    "Legacy agent grant id migration attempt {attempt}/3 left 1 rows unmigrated"
                )
    }));
    assert_eq!(
        row_agents(&db.pool, "p1").await,
        Some(vec![ALPHA_LEGACY.to_owned()])
    );
    db.close().await;
}

#[tokio::test(start_paused = true)]
async fn database_errors_are_logged_and_swallowed_after_three_attempts() {
    let registry = loaded(&[ALPHA_ENTRY]);
    let store = PostgresStore { pool: None };
    let (logs, _guard) = capture();
    let started = Instant::now();
    registry.run_grant_id_migration(&store).await;
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    let logged = messages(&logs);
    assert_eq!(logged.len(), 3);
    let failure = "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";
    assert!(logged.iter().zip(1..=3).all(|(entry, attempt)| {
        entry.level == Level::WARN
            && entry.message
                == format!("Legacy agent grant id migration attempt {attempt}/3 failed: {failure}")
    }));
}

#[tokio::test(start_paused = true)]
async fn a_missed_attempt_is_retried_until_a_clean_pass() {
    let schema = format!("{SCHEMA}{SKIP_FIRST_TRIGGER}");
    let db = TestDatabase::start(&schema).await;
    insert_row(&db.pool, "p1", &[ALPHA_LEGACY.to_owned()]).await;
    let registry = loaded(&[ALPHA_ENTRY]);
    let (logs, _guard) = capture();
    let started = Instant::now();
    registry.run_grant_id_migration(&db.store()).await;
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert_eq!(
        messages(&logs),
        vec![
            Logged {
                level: Level::WARN,
                message: String::from(
                    "Legacy agent grant id migration attempt 1/3 left 1 rows unmigrated"
                ),
            },
            Logged {
                level: Level::INFO,
                message: String::from(
                    "Rewrote 1 object_permission rows from legacy config agent ids"
                ),
            },
        ]
    );
    assert_eq!(
        row_agents(&db.pool, "p1").await,
        Some(vec![ALPHA_STABLE.to_owned()])
    );
    db.close().await;
}
