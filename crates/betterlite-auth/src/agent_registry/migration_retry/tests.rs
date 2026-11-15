#![cfg(test)]

use std::cell::Cell;
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
use crate::agent_registry::GrantMigrationResult;
use crate::agent_registry::tests::support::{ALPHA_ENTRY, loaded};

use super::retry_migration;

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
    let (logs, _guard) = capture();
    let calls = Cell::new(0_usize);
    let started = Instant::now();
    retry_migration(|| {
        calls.set(calls.get() + 1);
        async {
            Ok(GrantMigrationResult {
                rewritten: 1,
                missed: 0,
            })
        }
    })
    .await;
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(calls.get(), 1);
    assert_eq!(
        messages(&logs),
        vec![Logged {
            level: Level::INFO,
            message: String::from("Rewrote 1 object_permission rows from legacy config agent ids"),
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn missed_rows_are_warned_three_times_with_a_pause_each() {
    let (logs, _guard) = capture();
    let calls = Cell::new(0_usize);
    let started = Instant::now();
    retry_migration(|| {
        calls.set(calls.get() + 1);
        async {
            Ok(GrantMigrationResult {
                rewritten: 0,
                missed: 1,
            })
        }
    })
    .await;
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(calls.get(), 3);
    let logged = messages(&logs);
    assert_eq!(logged.len(), 3);
    assert!(logged.iter().zip(1_u32..=3_u32).all(|(entry, attempt)| {
        entry.level == Level::WARN
            && entry.message
                == format!(
                    "Legacy agent grant id migration attempt {attempt}/3 left 1 rows unmigrated"
                )
    }));
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
    assert!(logged.iter().zip(1_u32..=3_u32).all(|(entry, attempt)| {
        entry.level == Level::WARN
            && entry.message
                == format!("Legacy agent grant id migration attempt {attempt}/3 failed: {failure}")
    }));
}

#[tokio::test(start_paused = true)]
async fn a_missed_attempt_is_retried_until_a_clean_pass() {
    let (logs, _guard) = capture();
    let calls = Cell::new(0_usize);
    let started = Instant::now();
    retry_migration(|| {
        let attempt = calls.get() + 1;
        calls.set(attempt);
        let result = match attempt {
            1 => GrantMigrationResult {
                rewritten: 0,
                missed: 1,
            },
            _ => GrantMigrationResult {
                rewritten: 1,
                missed: 0,
            },
        };
        async move { Ok(result) }
    })
    .await;
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert_eq!(calls.get(), 2);
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
}
