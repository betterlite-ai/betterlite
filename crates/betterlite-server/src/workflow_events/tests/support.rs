use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::workflow_events::WorkflowEventsState;
use crate::workflow_events::models::{EventCreateData, MessageCreateData};
use crate::workflow_events::router::router;
use crate::workflow_events::store::{WorkflowEventStore, WriteError};
use crate::workflow_runs::models::{
    RunCreateData, RunUpdateData, RunWhere, SequenceRow, SequenceTable, WorkflowEventRow,
    WorkflowMessageRow, WorkflowRunRow, WorkflowRunStatus,
};
use crate::workflow_runs::store::{StoreError, WorkflowRunStore};

const BASE_SECONDS: i64 = 1_700_000_000;
const READ_FAILURE: &str = "connection reset";
const WRITE_FAILURE: &str = "the event write rolled back";
const LIST_FAILURE: &str = "the event read failed";
// a competing writer lands this far ahead, so only a fresh MAX+1 reaches past it
const COMPETING_GAP: i32 = 2;

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct DetailReply {
    pub(super) detail: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ValidationReply {
    pub(super) detail: Vec<IssueReply>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueReply {
    #[serde(rename = "type")]
    pub(super) issue_type: String,
    pub(super) loc: Vec<LocReply>,
    pub(super) msg: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
pub(super) enum LocReply {
    Name(String),
    Index(usize),
}

#[derive(Debug, Deserialize)]
pub(super) struct EventListReply {
    pub(super) events: Vec<WorkflowEventRow>,
    pub(super) count: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct MessageListReply {
    pub(super) messages: Vec<WorkflowMessageRow>,
    pub(super) count: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorReply {
    pub(super) error: ErrorBody,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorBody {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) error_type: String,
    pub(super) param: Option<String>,
    pub(super) code: String,
}

// the body LiteLLM's unhandled-exception handler returns when the run lookup itself fails
pub(super) fn assert_unhandled_failure(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let parsed: ErrorReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(parsed.error.message, "Internal server error");
    assert_eq!(parsed.error.error_type, "internal_server_error");
    assert!(parsed.error.param.is_none());
    assert_eq!(parsed.error.code, "500");
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Attempt {
    pub(super) table: SequenceTable,
    pub(super) sequence_number: i32,
    pub(super) new_status: Option<WorkflowRunStatus>,
}

#[derive(Clone, Copy)]
pub(super) enum FailureKind {
    Read,
    Write,
    List,
}

#[derive(Clone, Copy, Default)]
struct Failure {
    read: Option<&'static str>,
    write: Option<&'static str>,
    list: Option<&'static str>,
}

#[derive(Default)]
struct Data {
    runs: Vec<WorkflowRunRow>,
    events: Vec<WorkflowEventRow>,
    messages: BTreeMap<String, Vec<WorkflowMessageRow>>,
    attempts: Vec<Attempt>,
    event_writes: Vec<EventCreateData>,
    message_writes: Vec<MessageCreateData>,
    status_updates: Vec<(String, WorkflowRunStatus)>,
    lists: Vec<(SequenceTable, String, u16)>,
    event_collisions: u32,
    message_collisions: u32,
}

#[derive(Default)]
pub(super) struct FakeStore {
    data: Mutex<Data>,
    failure: Failure,
}

fn at() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(BASE_SECONDS, 0).expect("base time")
}

fn parse_json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("stored json")
}

fn run_row(run_id: &str, owner: Option<&str>) -> WorkflowRunRow {
    WorkflowRunRow {
        run_id: run_id.to_owned(),
        session_id: format!("session-{run_id}"),
        workflow_type: "review".to_owned(),
        status: "running".to_owned(),
        created_by: owner.map(str::to_owned),
        created_at: at(),
        updated_at: at(),
        input: None,
        output: None,
        metadata: None,
        events: None,
        messages: None,
    }
}

fn event_row(
    run_id: &str,
    event_type: &str,
    step_name: &str,
    sequence_number: i32,
    stored: Option<JsonData>,
) -> WorkflowEventRow {
    WorkflowEventRow {
        event_id: format!("event-{run_id}-{sequence_number}"),
        run_id: run_id.to_owned(),
        event_type: event_type.to_owned(),
        step_name: step_name.to_owned(),
        sequence_number,
        data: stored,
        created_at: at(),
        run: None,
    }
}

// the messages of a run are kept in ascending sequence order, as the ordered read would return them
fn push_message(data: &mut Data, message: &MessageCreateData) -> WorkflowMessageRow {
    let row = WorkflowMessageRow {
        message_id: format!("message-{}-{}", message.run_id, message.sequence_number),
        run_id: message.run_id.clone(),
        role: message.role.clone(),
        content: message.content.clone(),
        sequence_number: message.sequence_number,
        session_id: message.session_id.clone(),
        created_at: at(),
        run: None,
    };
    let rows = data.messages.entry(row.run_id.clone()).or_default();
    rows.push(row.clone());
    rows.sort_by_key(|held| held.sequence_number);
    row
}

impl FakeStore {
    pub(super) fn failing_as(kind: FailureKind) -> Self {
        let mut failure = Failure::default();
        match kind {
            FailureKind::Read => failure.read = Some(READ_FAILURE),
            FailureKind::Write => failure.write = Some(WRITE_FAILURE),
            FailureKind::List => failure.list = Some(LIST_FAILURE),
        }
        Self {
            data: Mutex::default(),
            failure,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Data> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn check(message: Option<&'static str>) -> Result<(), StoreError> {
        let Some(text) = message else {
            return Ok(());
        };
        Err(StoreError::Failed {
            message: text.to_owned(),
        })
    }

    fn unsupported(action: &str) -> StoreError {
        StoreError::Failed {
            message: format!("the events fake does not {action}"),
        }
    }

    // the attempt is recorded first; a collision stands in for a competing writer that already took a later number
    fn record_attempt(
        &self,
        table: SequenceTable,
        sequence_number: i32,
        new_status: Option<WorkflowRunStatus>,
    ) -> (MutexGuard<'_, Data>, bool) {
        let mut data = self.lock();
        data.attempts.push(Attempt {
            table,
            sequence_number,
            new_status,
        });
        let collisions = match table {
            SequenceTable::Events => &mut data.event_collisions,
            SequenceTable::Messages => &mut data.message_collisions,
        };
        let collides = *collisions > 0;
        *collisions = collisions.saturating_sub(1);
        (data, collides)
    }

    // a list call fails before it is recorded, as a read that never reached the database
    fn record_list(
        &self,
        table: SequenceTable,
        run_id: &str,
        take: u16,
    ) -> Result<MutexGuard<'_, Data>, StoreError> {
        Self::check(self.failure.list)?;
        let mut data = self.lock();
        data.lists.push((table, run_id.to_owned(), take));
        Ok(data)
    }

    pub(super) fn seed_run(&self, owner: Option<&str>) -> String {
        let mut data = self.lock();
        let run_id = format!("run-{}", data.runs.len());
        data.runs.push(run_row(&run_id, owner));
        run_id
    }

    pub(super) fn seed_event(&self, run_id: &str, sequence_number: i32) {
        let row = event_row(run_id, "step.started", "build", sequence_number, None);
        self.lock().events.push(row);
    }

    pub(super) fn seed_message(&self, run_id: &str, sequence_number: i32) {
        let message = MessageCreateData {
            run_id: run_id.to_owned(),
            role: "user".to_owned(),
            content: "hello".to_owned(),
            sequence_number,
            session_id: None,
        };
        push_message(&mut self.lock(), &message);
    }

    pub(super) fn collide_events(&self, times: u32) {
        self.lock().event_collisions = times;
    }

    pub(super) fn collide_messages(&self, times: u32) {
        self.lock().message_collisions = times;
    }

    pub(super) fn attempts(&self) -> Vec<Attempt> {
        self.lock().attempts.clone()
    }

    pub(super) fn event_writes(&self) -> Vec<EventCreateData> {
        self.lock().event_writes.clone()
    }

    pub(super) fn message_writes(&self) -> Vec<MessageCreateData> {
        self.lock().message_writes.clone()
    }

    pub(super) fn status_updates(&self) -> Vec<(String, WorkflowRunStatus)> {
        self.lock().status_updates.clone()
    }

    pub(super) fn lists(&self) -> Vec<(SequenceTable, String, u16)> {
        self.lock().lists.clone()
    }

    pub(super) fn run_status(&self, run_id: &str) -> Option<String> {
        self.lock()
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
            .map(|run| run.status.clone())
    }
}

#[async_trait]
impl WorkflowRunStore for FakeStore {
    async fn create(&self, _data: &RunCreateData) -> Result<WorkflowRunRow, StoreError> {
        Err(Self::unsupported("create runs"))
    }

    async fn find_many(
        &self,
        _filter: &RunWhere,
        _take: u8,
    ) -> Result<Vec<WorkflowRunRow>, StoreError> {
        Err(Self::unsupported("list runs"))
    }

    async fn find_unique(&self, run_id: &str) -> Result<Option<WorkflowRunRow>, StoreError> {
        Self::check(self.failure.read)?;
        Ok(self
            .lock()
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
            .cloned())
    }

    async fn find_unique_with_latest_event(
        &self,
        _run_id: &str,
    ) -> Result<Option<WorkflowRunRow>, StoreError> {
        Err(Self::unsupported("read the latest event"))
    }

    async fn update(
        &self,
        _run_id: &str,
        _data: &RunUpdateData,
    ) -> Result<Option<WorkflowRunRow>, StoreError> {
        Err(Self::unsupported("update a run here"))
    }

    async fn latest_sequence(
        &self,
        run_id: &str,
        table: SequenceTable,
    ) -> Result<Option<SequenceRow>, StoreError> {
        Self::check(self.failure.read)?;
        let data = self.lock();
        let sequence_number = match table {
            SequenceTable::Events => data
                .events
                .iter()
                .filter(|event| event.run_id == run_id)
                .map(|event| event.sequence_number)
                .max(),
            SequenceTable::Messages => data
                .messages
                .get(run_id)
                .and_then(|held| held.last())
                .map(|message| message.sequence_number),
        };
        drop(data);
        Ok(sequence_number.map(|number| SequenceRow {
            sequence_number: number,
        }))
    }
}

#[async_trait]
impl WorkflowEventStore for FakeStore {
    async fn append_event(
        &self,
        data: &EventCreateData,
        new_status: Option<WorkflowRunStatus>,
    ) -> Result<WorkflowEventRow, WriteError> {
        Self::check(self.failure.write)?;
        let (mut inner, collides) =
            self.record_attempt(SequenceTable::Events, data.sequence_number, new_status);
        if collides {
            let taken = event_row(
                &data.run_id,
                &data.event_type,
                &data.step_name,
                data.sequence_number.saturating_add(COMPETING_GAP),
                None,
            );
            inner.events.push(taken);
            return Err(WriteError::SequenceTaken);
        }
        inner.event_writes.push(data.clone());
        let row = event_row(
            &data.run_id,
            &data.event_type,
            &data.step_name,
            data.sequence_number,
            data.data.as_deref().map(parse_json),
        );
        inner.events.push(row.clone());
        if let Some(status) = new_status {
            if let Some(run) = inner.runs.iter_mut().find(|run| run.run_id == data.run_id) {
                run.status = status.to_string();
            }
            inner.status_updates.push((data.run_id.clone(), status));
        }
        drop(inner);
        Ok(row)
    }

    async fn append_message(
        &self,
        data: &MessageCreateData,
    ) -> Result<WorkflowMessageRow, WriteError> {
        Self::check(self.failure.write)?;
        let (mut inner, collides) =
            self.record_attempt(SequenceTable::Messages, data.sequence_number, None);
        if collides {
            let ahead = MessageCreateData {
                run_id: data.run_id.clone(),
                role: data.role.clone(),
                content: data.content.clone(),
                sequence_number: data.sequence_number.saturating_add(COMPETING_GAP),
                session_id: data.session_id.clone(),
            };
            push_message(&mut inner, &ahead);
            return Err(WriteError::SequenceTaken);
        }
        inner.message_writes.push(data.clone());
        let row = push_message(&mut inner, data);
        drop(inner);
        Ok(row)
    }

    async fn events_for_run(
        &self,
        run_id: &str,
        take: u16,
    ) -> Result<Vec<WorkflowEventRow>, StoreError> {
        let data = self.record_list(SequenceTable::Events, run_id, take)?;
        let mut rows: Vec<WorkflowEventRow> = data
            .events
            .iter()
            .filter(|event| event.run_id == run_id)
            .cloned()
            .collect();
        drop(data);
        rows.sort_by_key(|event| event.sequence_number);
        rows.truncate(usize::from(take));
        Ok(rows)
    }

    async fn messages_for_run(
        &self,
        run_id: &str,
        take: u16,
    ) -> Result<Vec<WorkflowMessageRow>, StoreError> {
        let data = self.record_list(SequenceTable::Messages, run_id, take)?;
        Ok(data.messages.get(run_id).map_or_else(Vec::new, |held| {
            held.iter().take(usize::from(take)).cloned().collect()
        }))
    }
}

pub(super) fn state(store: Option<&Arc<FakeStore>>) -> WorkflowEventsState {
    let Some(fake) = store else {
        return WorkflowEventsState {
            runs: None,
            events: None,
        };
    };
    let runs: Arc<dyn WorkflowRunStore> = Arc::<FakeStore>::clone(fake);
    let events: Arc<dyn WorkflowEventStore> = Arc::<FakeStore>::clone(fake);
    WorkflowEventsState {
        runs: Some(runs),
        events: Some(events),
    }
}

pub(super) fn principal(subject: &str, roles: Vec<Role>, token: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef {
            key_id: None,
            token_id: token.map(str::to_owned),
        },
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

pub(super) fn master() -> Principal {
    master_caller().principal
}

pub(super) fn proxy_admin(token: &str) -> Principal {
    principal("virtual-key", vec![Role::PlatformAdmin], Some(token))
}

pub(super) fn viewer(token: &str) -> Principal {
    principal("virtual-key", vec![Role::PlatformViewer], Some(token))
}

pub(super) fn member(token: &str) -> Principal {
    principal("virtual-key", vec![Role::TeamMember], Some(token))
}

pub(super) fn member_without_key() -> Principal {
    principal("virtual-key", vec![Role::TeamMember], None)
}

fn request(method: Method, uri: &str, caller: Option<&Principal>, body: &str) -> Request<Body> {
    let mut built = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request");
    if let Some(identity) = caller {
        built.extensions_mut().insert(AuthContext {
            principal: identity.clone(),
        });
    }
    built
}

pub(super) async fn send(store: Option<&Arc<FakeStore>>, request: Request<Body>) -> Reply {
    let application = router(state(store));
    let response = application.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    Reply {
        status,
        body: String::from_utf8(bytes.to_vec()).expect("utf8 body"),
    }
}

pub(super) async fn post(
    store: Option<&Arc<FakeStore>>,
    caller: Option<&Principal>,
    uri: &str,
    body: &str,
) -> Reply {
    send(store, request(Method::POST, uri, caller, body)).await
}

pub(super) async fn get(
    store: Option<&Arc<FakeStore>>,
    caller: Option<&Principal>,
    uri: &str,
) -> Reply {
    send(store, request(Method::GET, uri, caller, "")).await
}
