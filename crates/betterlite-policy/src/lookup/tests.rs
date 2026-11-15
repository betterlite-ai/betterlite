#![cfg(test)]

use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::pipeline::json_value::JsonValue;
use crate::records::db::PolicyDBResponse;
use crate::row_adapter::{
    PolicyClause, PolicyOrder, PolicyRow, PolicyTable, PolicyTableSource, policy_db_response,
};

use super::{PolicyLookupError, get_policy_by_id_from_db};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct FakeFailure(String);

#[derive(Clone)]
struct FakeTable {
    answer: Result<Option<PolicyRow>, String>,
    filters: Arc<Mutex<Vec<PolicyClause>>>,
}

impl FakeTable {
    fn answering(answer: Result<Option<PolicyRow>, String>) -> Self {
        Self {
            answer,
            filters: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn recorded(&self) -> Vec<PolicyClause> {
        self.filters.lock().unwrap().clone()
    }

    fn stored(&self) -> Result<PolicyRow, FakeFailure> {
        self.answer
            .clone()
            .map_err(FakeFailure)?
            .ok_or_else(|| FakeFailure("no row".to_owned()))
    }
}

impl PolicyTable<PolicyRow> for FakeTable {
    type Failure = FakeFailure;

    fn create(
        &self,
        _data: PolicyClause,
    ) -> impl Future<Output = Result<PolicyRow, FakeFailure>> + Send {
        ready(self.stored())
    }

    fn find_unique(
        &self,
        filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<PolicyRow>, FakeFailure>> + Send {
        self.filters.lock().unwrap().push(filter);
        ready(self.answer.clone().map_err(FakeFailure))
    }

    fn find_many(
        &self,
        _filter: Option<PolicyClause>,
        _order: Option<PolicyOrder>,
    ) -> impl Future<Output = Result<Vec<PolicyRow>, FakeFailure>> + Send {
        ready(
            self.answer
                .clone()
                .map_err(FakeFailure)
                .map(|row| row.into_iter().collect()),
        )
    }

    fn update(
        &self,
        _filter: PolicyClause,
        _data: PolicyClause,
    ) -> impl Future<Output = Result<PolicyRow, FakeFailure>> + Send {
        ready(self.stored())
    }

    fn update_many(
        &self,
        _filter: PolicyClause,
        _data: PolicyClause,
    ) -> impl Future<Output = Result<usize, FakeFailure>> + Send {
        ready(Ok(0))
    }

    fn delete(
        &self,
        _filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<PolicyRow>, FakeFailure>> + Send {
        ready(self.answer.clone().map_err(FakeFailure))
    }

    fn delete_many(
        &self,
        _filter: PolicyClause,
    ) -> impl Future<Output = Result<usize, FakeFailure>> + Send {
        ready(Ok(0))
    }
}

struct FakeSource {
    table: FakeTable,
}

impl PolicyTableSource for FakeSource {
    type Table = FakeTable;

    fn policy_table(&self) -> Self::Table {
        self.table.clone()
    }
}

fn timestamp(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn row() -> PolicyRow {
    PolicyRow {
        policy_id: "policy_1".to_owned(),
        policy_name: "healthcare".to_owned(),
        version_number: None,
        version_status: None,
        parent_version_id: None,
        is_latest: None,
        published_at: None,
        production_at: None,
        inherit: Some("baseline".to_owned()),
        description: None,
        guardrails_add: Some(vec!["hipaa".to_owned()]),
        guardrails_remove: None,
        condition: None,
        pipeline: None,
        created_at: timestamp("2024-01-01T00:00:00Z"),
        updated_at: timestamp("2024-01-02T00:00:00Z"),
        created_by: None,
        updated_by: None,
    }
}

fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

fn expected_filter() -> PolicyClause {
    PolicyClause::from([(
        "policy_id".to_owned(),
        JsonValue::String("policy_1".to_owned()),
    )])
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

struct ErrorCapture(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for ErrorCapture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &Level::ERROR {
            let mut visitor = MessageVisitor(String::new());
            event.record(&mut visitor);
            self.0.lock().unwrap().push(visitor.0);
        }
    }
}

fn capture_errors<F, T>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(ErrorCapture(Arc::clone(&messages)));
    let value = with_default(subscriber, run);
    let collected = messages.lock().unwrap().clone();
    (value, collected)
}

#[test]
fn lookup_returns_the_adapted_row_for_the_policy_id() {
    let source = FakeSource {
        table: FakeTable::answering(Ok(Some(row()))),
    };
    let (result, messages) =
        capture_errors(|| runtime().block_on(get_policy_by_id_from_db("policy_1", &source)));
    let found: Option<PolicyDBResponse> = result.unwrap();
    assert_eq!(found, Some(policy_db_response(&row())));
    assert_eq!(source.table.recorded(), [expected_filter()]);
    assert!(messages.is_empty());
}

#[test]
fn lookup_returns_none_when_the_row_is_missing() {
    let source = FakeSource {
        table: FakeTable::answering(Ok(None)),
    };
    let (result, messages) =
        capture_errors(|| runtime().block_on(get_policy_by_id_from_db("policy_1", &source)));
    assert_eq!(result.unwrap(), None);
    assert_eq!(source.table.recorded(), [expected_filter()]);
    assert!(messages.is_empty());
}

#[test]
fn lookup_logs_and_returns_the_database_failure() {
    let source = FakeSource {
        table: FakeTable::answering(Err("connection reset".to_owned())),
    };
    let (result, messages) =
        capture_errors(|| runtime().block_on(get_policy_by_id_from_db("policy_1", &source)));
    let error: PolicyLookupError<FakeFailure> = result.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting policy from DB: connection reset"
    );
    let PolicyLookupError::Database { source: failure } = error;
    assert_eq!(failure.to_string(), "connection reset");
    assert_eq!(messages, ["Error getting policy from DB: connection reset"]);
    assert_eq!(source.table.recorded(), [expected_filter()]);
}
