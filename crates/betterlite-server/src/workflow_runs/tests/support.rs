use std::cmp::Reverse;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::workflow_runs::WorkflowRunsState;
use crate::workflow_runs::models::{
    RunCreateData, RunUpdateData, RunWhere, SequenceRow, SequenceTable, StatusFilter,
    WorkflowEventRow, WorkflowRunRow,
};
use crate::workflow_runs::router::router;
use crate::workflow_runs::store::{StoreError, WorkflowRunStore};

const BASE_SECONDS: i64 = 1_700_000_000;

#[derive(Clone, Default)]
pub(super) struct Recorded {
    pub(super) created: Vec<RunCreateData>,
    pub(super) filters: Vec<(RunWhere, u8)>,
    pub(super) updates: Vec<(String, RunUpdateData)>,
}

#[derive(Default)]
pub(super) struct FakeState {
    runs: Vec<WorkflowRunRow>,
    events: Vec<WorkflowEventRow>,
    message_sequences: Vec<(String, i32)>,
    recorded: Recorded,
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub(super) state: Mutex<FakeState>,
    pub(super) failing: bool,
    pub(super) vanish_on_update: bool,
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct DetailReply {
    pub(super) detail: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListReply {
    pub(super) runs: Vec<WorkflowRunRow>,
    pub(super) count: usize,
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
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
pub(super) enum LocReply {
    Name(String),
    Index(usize),
}

fn at(offset_seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(BASE_SECONDS, 0)
        .expect("base time")
        .checked_add_signed(Duration::seconds(offset_seconds))
        .expect("offset time")
}

impl FakeStore {
    pub(super) fn seed_run(
        &self,
        owner: Option<&str>,
        workflow_type: &str,
        status: &str,
    ) -> String {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let number = state.runs.len();
        let run_id = format!("run-{number}");
        let offset = i64::try_from(number).expect("offset");
        state.runs.push(WorkflowRunRow {
            run_id: run_id.clone(),
            session_id: format!("session-{number}"),
            workflow_type: workflow_type.to_owned(),
            status: status.to_owned(),
            created_by: owner.map(str::to_owned),
            created_at: at(offset),
            updated_at: at(offset),
            input: None,
            output: None,
            metadata: None,
            events: None,
            messages: None,
        });
        drop(state);
        run_id
    }

    pub(super) fn seed_event(&self, run_id: &str, sequence_number: i32) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.events.push(WorkflowEventRow {
            event_id: format!("event-{run_id}-{sequence_number}"),
            run_id: run_id.to_owned(),
            event_type: "step.started".to_owned(),
            step_name: "build".to_owned(),
            sequence_number,
            data: None,
            created_at: at(0),
            run: None,
        });
        drop(state);
    }

    pub(super) fn seed_message(&self, run_id: &str, sequence_number: i32) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .message_sequences
            .push((run_id.to_owned(), sequence_number));
    }

    pub(super) fn seed_sequences(&self, run_id: &str, events: &[i32], messages: &[i32]) {
        for number in events {
            self.seed_event(run_id, *number);
        }
        for number in messages {
            self.seed_message(run_id, *number);
        }
    }

    pub(super) fn recorded(&self) -> Recorded {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.recorded.clone()
    }

    fn check(&self) -> Result<(), StoreError> {
        if self.failing {
            return Err(StoreError::Failed {
                message: "connection reset".to_owned(),
            });
        }
        Ok(())
    }
}

fn matches_filter(row: &WorkflowRunRow, filter: &RunWhere) -> bool {
    let type_matches = filter
        .workflow_type
        .as_ref()
        .is_none_or(|expected| *expected == row.workflow_type);
    let status_matches = match &filter.status {
        None => true,
        Some(StatusFilter::Equals(expected)) => *expected == row.status,
        Some(StatusFilter::In(options)) => options.contains(&row.status),
    };
    let owner_matches = filter
        .created_by
        .as_ref()
        .is_none_or(|expected| row.created_by.as_ref() == Some(expected));
    type_matches && status_matches && owner_matches
}

fn parse_json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("stored json")
}

#[async_trait]
impl WorkflowRunStore for FakeStore {
    async fn create(&self, data: &RunCreateData) -> Result<WorkflowRunRow, StoreError> {
        self.check()?;
        let run_id = self.seed_run(data.created_by.as_deref(), &data.workflow_type, "pending");
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.recorded.created.push(data.clone());
        let row = state
            .runs
            .iter_mut()
            .find(|candidate| candidate.run_id == run_id)
            .expect("seeded run");
        row.input = data.input.as_deref().map(parse_json);
        row.metadata = data.metadata.as_deref().map(parse_json);
        let created = row.clone();
        drop(state);
        Ok(created)
    }

    async fn find_many(
        &self,
        filter: &RunWhere,
        take: u8,
    ) -> Result<Vec<WorkflowRunRow>, StoreError> {
        self.check()?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.recorded.filters.push((filter.clone(), take));
        let mut rows: Vec<WorkflowRunRow> = state
            .runs
            .iter()
            .filter(|row| matches_filter(row, filter))
            .cloned()
            .collect();
        drop(state);
        rows.sort_by_key(|left| Reverse(left.created_at));
        rows.truncate(usize::from(take));
        Ok(rows)
    }

    async fn find_unique(&self, run_id: &str) -> Result<Option<WorkflowRunRow>, StoreError> {
        self.check()?;
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state.runs.iter().find(|row| row.run_id == run_id).cloned())
    }

    async fn find_unique_with_latest_event(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowRunRow>, StoreError> {
        self.check()?;
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(mut run) = state.runs.iter().find(|row| row.run_id == run_id).cloned() else {
            return Ok(None);
        };
        run.events = Some(
            state
                .events
                .iter()
                .filter(|event| event.run_id == run_id)
                .max_by_key(|event| event.sequence_number)
                .cloned()
                .into_iter()
                .collect(),
        );
        drop(state);
        Ok(Some(run))
    }

    async fn update(
        &self,
        run_id: &str,
        data: &RunUpdateData,
    ) -> Result<Option<WorkflowRunRow>, StoreError> {
        self.check()?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .recorded
            .updates
            .push((run_id.to_owned(), data.clone()));
        if self.vanish_on_update {
            return Ok(None);
        }
        let Some(row) = state.runs.iter_mut().find(|row| row.run_id == run_id) else {
            return Ok(None);
        };
        if let Some(status) = data.status {
            row.status = status.to_string();
        }
        if let Some(output) = &data.output {
            row.output = Some(parse_json(output));
        }
        if let Some(metadata) = &data.metadata {
            row.metadata = Some(parse_json(metadata));
        }
        let updated = row.clone();
        drop(state);
        Ok(Some(updated))
    }

    async fn latest_sequence(
        &self,
        run_id: &str,
        table: SequenceTable,
    ) -> Result<Option<SequenceRow>, StoreError> {
        self.check()?;
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let latest = match table {
            SequenceTable::Events => state
                .events
                .iter()
                .filter(|event| event.run_id == run_id)
                .map(|event| event.sequence_number)
                .max(),
            SequenceTable::Messages => state
                .message_sequences
                .iter()
                .filter(|(owner, _number)| owner == run_id)
                .map(|(_owner, number)| *number)
                .max(),
        };
        drop(state);
        Ok(latest.map(|sequence_number| SequenceRow { sequence_number }))
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

pub(super) async fn call(
    fake_store: Option<&Arc<FakeStore>>,
    caller: Option<&Principal>,
    method: Method,
    uri: &str,
    body: &str,
) -> Reply {
    let store =
        fake_store.map(|fake| -> Arc<dyn WorkflowRunStore> { Arc::<FakeStore>::clone(fake) });
    let application = router(WorkflowRunsState { store });
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request");
    if let Some(identity) = caller {
        request.extensions_mut().insert(AuthContext {
            principal: identity.clone(),
        });
    }
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
