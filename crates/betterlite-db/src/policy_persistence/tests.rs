#![cfg(test)]

use std::fmt::Debug;
use std::future::{Future, ready};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::pipeline::json_value::JsonValue;
use betterlite_policy::records::JsonObject;
use betterlite_policy::registry::PolicyRegistry;
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyOrder, PolicyRow, PolicySort, PolicyTable, PolicyTableSource,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::attachment_row::{AttachmentRow, AttachmentTableSource};
use super::{AttachmentStore, PolicyStore};

pub(super) const NOW: &str = "2026-03-01T10:00:00Z";

pub(super) fn fixed_now() -> DateTime<Utc> {
    NOW.parse().unwrap()
}

#[derive(Debug, thiserror::Error)]
#[error("table refused {0}")]
pub(super) struct FakeFailure(&'static str);

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Call {
    Create(PolicyClause),
    FindUnique(PolicyClause),
    FindMany(Option<PolicyClause>, Option<PolicyOrder>),
    Update(PolicyClause, PolicyClause),
    Delete(PolicyClause),
}

struct TableState {
    rows: Vec<JsonObject>,
    calls: Vec<Call>,
    failing: Vec<&'static str>,
}

pub(super) struct FakeTable<Row> {
    state: Arc<Mutex<TableState>>,
    id_field: &'static str,
    row: PhantomData<fn() -> Row>,
}

impl<Row> Clone for FakeTable<Row> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            id_field: self.id_field,
            row: PhantomData,
        }
    }
}

pub(super) fn object<Value>(value: &Value) -> JsonObject
where
    Value: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn decode<Row>(row: &JsonObject) -> Row
where
    Row: DeserializeOwned,
{
    serde_json::from_str(&serde_json::to_string(row).unwrap()).unwrap()
}

pub(super) fn id_filter(field: &str, id: &str) -> PolicyClause {
    PolicyClause::from([(field.to_owned(), JsonValue::String(id.to_owned()))])
}

fn matches(row: &JsonObject, filter: &PolicyClause) -> bool {
    filter.iter().all(|(field, expected)| {
        let actual = row.get(field).unwrap_or(&JsonValue::Null);
        let JsonValue::Object(operators) = expected else {
            return actual == expected;
        };
        matches!(operators.get("in"), Some(JsonValue::Array(options)) if options.contains(actual))
    })
}

fn ordered(
    rows: &[JsonObject],
    filter: &PolicyClause,
    order: Option<&PolicyOrder>,
) -> Vec<JsonObject> {
    let mut found: Vec<JsonObject> = rows
        .iter()
        .filter(|row| matches(row, filter))
        .cloned()
        .collect();
    for (field, direction) in order.into_iter().flatten() {
        found.sort_by_key(|row| serde_json::to_string(&row.get(field)).unwrap());
        if *direction == PolicySort::Desc {
            found.reverse();
        }
    }
    found
}

impl Call {
    const fn method(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::FindUnique(_) => "find_unique",
            Self::FindMany(..) => "find_many",
            Self::Update(..) => "update",
            Self::Delete(_) => "delete",
        }
    }
}

impl<Row> FakeTable<Row> {
    fn holding<Stored>(id_field: &'static str, rows: &[Stored]) -> Self
    where
        Stored: Serialize,
    {
        let state = TableState {
            rows: rows.iter().map(object).collect(),
            calls: Vec::new(),
            failing: Vec::new(),
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            id_field,
            row: PhantomData,
        }
    }

    pub(super) fn fail(&self, method: &'static str) {
        self.state.lock().unwrap().failing.push(method);
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    pub(super) fn rows(&self) -> Vec<JsonObject> {
        self.state.lock().unwrap().rows.clone()
    }

    fn answer(&self, call: Call) -> Result<Vec<Row>, FakeFailure>
    where
        Row: DeserializeOwned,
    {
        let mut state = self.state.lock().unwrap();
        state.calls.push(call.clone());
        if state.failing.contains(&call.method()) {
            return Err(FakeFailure(call.method()));
        }
        let rows = &mut state.rows;
        let produced: Vec<JsonObject> = match call {
            Call::Create(mut created) => {
                let id = format!("{}-{}", self.id_field, rows.len());
                created.insert(self.id_field.to_owned(), JsonValue::String(id));
                rows.push(created.clone());
                vec![created]
            }
            Call::FindUnique(filter) => rows
                .iter()
                .filter(|row| matches(row, &filter))
                .take(1)
                .cloned()
                .collect(),
            Call::FindMany(filter, order) => {
                ordered(rows, &filter.unwrap_or_default(), order.as_ref())
            }
            Call::Update(filter, data) => {
                let updated = rows.iter_mut().find(|row| matches(row, &filter)).unwrap();
                updated.extend(data);
                vec![updated.clone()]
            }
            Call::Delete(filter) => {
                let position = rows.iter().position(|row| matches(row, &filter));
                position
                    .map(|index| rows.remove(index))
                    .into_iter()
                    .collect()
            }
        };
        Ok(produced.iter().map(decode).collect())
    }
}

fn single<Row>(mut rows: Vec<Row>) -> Row {
    rows.remove(0)
}

impl<Row: DeserializeOwned + Send + 'static> PolicyTable<Row> for FakeTable<Row> {
    type Failure = FakeFailure;

    fn create(&self, data: PolicyClause) -> impl Future<Output = Result<Row, FakeFailure>> + Send {
        ready(self.answer(Call::Create(data)).map(single))
    }

    fn find_unique(
        &self,
        filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<Row>, FakeFailure>> + Send {
        ready(
            self.answer(Call::FindUnique(filter))
                .map(|mut rows| rows.pop()),
        )
    }

    fn find_many(
        &self,
        filter: Option<PolicyClause>,
        order: Option<PolicyOrder>,
    ) -> impl Future<Output = Result<Vec<Row>, FakeFailure>> + Send {
        ready(self.answer(Call::FindMany(filter, order)))
    }

    fn update(
        &self,
        filter: PolicyClause,
        data: PolicyClause,
    ) -> impl Future<Output = Result<Row, FakeFailure>> + Send {
        ready(self.answer(Call::Update(filter, data)).map(single))
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
        filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<Row>, FakeFailure>> + Send {
        ready(self.answer(Call::Delete(filter)).map(|mut rows| rows.pop()))
    }

    fn delete_many(
        &self,
        _filter: PolicyClause,
    ) -> impl Future<Output = Result<usize, FakeFailure>> + Send {
        ready(Ok(0))
    }
}

pub(super) struct FakeClient {
    pub(super) policies: FakeTable<PolicyRow>,
    pub(super) attachments: FakeTable<AttachmentRow>,
}

impl PolicyTableSource for FakeClient {
    type Table = FakeTable<PolicyRow>;

    fn policy_table(&self) -> Self::Table {
        self.policies.clone()
    }
}

impl AttachmentTableSource for FakeClient {
    type Table = FakeTable<AttachmentRow>;

    fn attachment_table(&self) -> Self::Table {
        self.attachments.clone()
    }
}

pub(super) struct Fixture {
    pub(super) client: FakeClient,
    pub(super) policies: Mutex<PolicyRegistry>,
    pub(super) attachments: Mutex<AttachmentRegistry>,
}

impl Fixture {
    pub(super) fn new(policy_rows: &[PolicyRow], attachment_rows: &[AttachmentRow]) -> Self {
        let client = FakeClient {
            policies: FakeTable::holding("policy_id", policy_rows),
            attachments: FakeTable::holding("attachment_id", attachment_rows),
        };
        Self {
            client,
            policies: Mutex::default(),
            attachments: Mutex::default(),
        }
    }

    pub(super) fn policy_store(&self) -> PolicyStore<'_, FakeClient> {
        PolicyStore {
            client: &self.client,
            registry: &self.policies,
            now: fixed_now,
        }
    }

    pub(super) fn attachment_store(&self) -> AttachmentStore<'_, FakeClient> {
        AttachmentStore {
            client: &self.client,
            registry: &self.attachments,
            now: fixed_now,
        }
    }
}

pub(super) fn policy_row(id: &str, name: &str, status: &str, created: &str) -> PolicyRow {
    let text = format!(
        r#"{{"policy_id": "{id}", "policy_name": "{name}", "version_status": "{status}", "guardrails_add": ["{name}-guard"], "created_at": "{created}", "updated_at": "{created}"}}"#
    );
    serde_json::from_str(&text).unwrap()
}

pub(super) fn attachment_row(id: &str, name: &str, created: &str) -> AttachmentRow {
    let text = format!(
        r#"{{"attachment_id": "{id}", "policy_name": "{name}", "teams": ["team-a"], "keys": [], "models": [], "tags": [], "is_default": false, "created_at": "{created}", "updated_at": "{created}"}}"#
    );
    serde_json::from_str(&text).unwrap()
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<(Level, String)>>>);

impl CapturedLogs {
    pub(super) fn install() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = set_default(tracing_subscriber::registry().with(logs.clone()));
        (logs, guard)
    }

    pub(super) fn at(&self, level: Level) -> Vec<String> {
        let entries = self.0.lock().unwrap();
        entries
            .iter()
            .filter(|entry| entry.0 == level)
            .map(|entry| entry.1.clone())
            .collect()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), message.0));
    }
}
