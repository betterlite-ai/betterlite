#![cfg(test)]

use std::cmp::Reverse;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex as StdMutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::mutation::prompt_registry_mutation::callbacks::PromptCallbackManager;
use betterlite_prompts::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use betterlite_prompts::registry::prompt_endpoint_contracts::rows::{
    PromptRow, PromptRowData, PromptRowJson,
};
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptRowFilter, PromptRowQuery, PromptRowUpdate, PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry::InMemoryPromptRegistry;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::Mutex;
use tower::ServiceExt as _;

use super::super::routes;
use super::super::state::PromptWriteState;
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;

pub(super) type SharedRows = Arc<StdMutex<Vec<PromptRow>>>;
pub(super) type SharedCalls = Arc<StdMutex<Vec<&'static str>>>;

#[derive(Debug, Error)]
#[error("the fake table failed")]
pub(super) struct FakeFailure;

pub(super) struct FakeTable {
    rows: SharedRows,
    fail_deletes: bool,
}

#[derive(Clone, Copy)]
pub(super) enum TableMode {
    Absent,
    Working,
    FailingDeletes,
}

pub(super) struct RecordingCallbacks {
    pub(super) calls: SharedCalls,
}

impl PromptCallbackManager for RecordingCallbacks {
    fn add_litellm_callback(&mut self, _callback: &PromptManagementClient) {
        self.calls.lock().expect("calls").push("added");
    }

    fn remove_callback_from_all_lists(&mut self, _callback: &PromptManagementClient) {
        self.calls.lock().expect("calls").push("removed");
    }
}

fn row_matches(row: &PromptRow, filter: &PromptRowFilter) -> bool {
    filter.id.as_ref().is_none_or(|id| *id == row.id)
        && filter
            .prompt_id
            .as_ref()
            .is_none_or(|prompt_id| *prompt_id == row.prompt_id)
        && filter.version.is_none_or(|version| version == row.version)
        && filter
            .environment
            .as_ref()
            .is_none_or(|environment| *environment == row.environment)
}

fn fixed_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
}

impl PromptTableActions for FakeTable {
    type Failure = FakeFailure;

    fn find_many(
        &self,
        query: PromptRowQuery,
    ) -> impl Future<Output = Result<Vec<PromptRow>, FakeFailure>> + Send {
        let mut found: Vec<PromptRow> = self
            .rows
            .lock()
            .expect("rows")
            .iter()
            .filter(|row| row_matches(row, &query.filter))
            .cloned()
            .collect();
        if !query.order.is_empty() {
            found.sort_by_key(|row| Reverse(row.version));
        }
        if let Some(take) = query.take {
            found.truncate(usize::try_from(take).expect("take"));
        }
        ready(Ok(found))
    }

    fn create(
        &self,
        data: PromptRowData,
    ) -> impl Future<Output = Result<PromptRow, FakeFailure>> + Send {
        let mut rows = self.rows.lock().expect("rows");
        let row = PromptRow {
            id: format!("row-{}", rows.len().saturating_add(1)),
            prompt_id: data.prompt_id,
            version: data.version,
            environment: data.environment,
            created_by: data.created_by,
            created_at: fixed_time(),
            updated_at: fixed_time(),
            litellm_params: data.litellm_params.expect("litellm_params"),
            prompt_info: data.prompt_info,
        };
        rows.push(row.clone());
        drop(rows);
        ready(Ok(row))
    }

    fn update(
        &self,
        filter: PromptRowFilter,
        data: PromptRowUpdate,
    ) -> impl Future<Output = Result<Option<PromptRow>, FakeFailure>> + Send {
        let updated = {
            let mut rows = self.rows.lock().expect("rows");
            rows.iter_mut()
                .find(|row| row_matches(row, &filter))
                .map(|row| {
                    row.litellm_params = data.litellm_params;
                    row.prompt_info = Some(data.prompt_info);
                    row.created_by = data.created_by.or_else(|| row.created_by.take());
                    row.clone()
                })
        };
        ready(Ok(updated))
    }

    fn delete_many(
        &self,
        filter: PromptRowFilter,
    ) -> impl Future<Output = Result<u64, FakeFailure>> + Send {
        if self.fail_deletes {
            return ready(Err(FakeFailure));
        }
        let mut rows = self.rows.lock().expect("rows");
        let before = rows.len();
        rows.retain(|row| !row_matches(row, &filter));
        let removed = u64::try_from(before.saturating_sub(rows.len())).expect("count");
        drop(rows);
        ready(Ok(removed))
    }
}

pub(super) fn context(subject: &str, roles: Vec<Role>, user_id: Option<&str>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: user_id.map(|id| UserIdentity {
                id: id.to_owned(),
                external_id: None,
                user_name: None,
                email: None,
                display_name: None,
            }),
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(super) fn master_key() -> AuthContext {
    master_caller()
}

pub(super) fn viewer() -> AuthContext {
    context("user-2", vec![Role::PlatformViewer], Some("bob"))
}

pub(super) fn proxy_admin() -> AuthContext {
    context("user-1", vec![Role::PlatformAdmin], Some("alice"))
}

pub(super) struct Harness {
    pub(super) rows: SharedRows,
    pub(super) registry: Arc<Mutex<InMemoryPromptRegistry>>,
    calls: SharedCalls,
    callbacks: Arc<Mutex<dyn PromptCallbackManager + Send>>,
    router: Router,
}

impl Harness {
    pub(super) fn new(context: AuthContext, mode: TableMode) -> Self {
        let rows = SharedRows::default();
        let registry = Arc::new(Mutex::new(InMemoryPromptRegistry::default()));
        let calls = SharedCalls::default();
        let callbacks: Arc<Mutex<dyn PromptCallbackManager + Send>> =
            Arc::new(Mutex::new(RecordingCallbacks {
                calls: Arc::clone(&calls),
            }));
        let table = match mode {
            TableMode::Absent => None,
            TableMode::Working => Some(FakeTable {
                rows: Arc::clone(&rows),
                fail_deletes: false,
            }),
            TableMode::FailingDeletes => Some(FakeTable {
                rows: Arc::clone(&rows),
                fail_deletes: true,
            }),
        };
        let state = PromptWriteState {
            table,
            registry: Arc::clone(&registry),
            callbacks: Arc::clone(&callbacks),
        };
        Self {
            rows,
            registry,
            calls,
            callbacks,
            router: routes(Arc::new(state)).layer(Extension(context)),
        }
    }

    pub(super) fn recorded_calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("calls").clone()
    }

    pub(super) async fn send(&self, method: Method, uri: &str, body: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .expect("request");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 0x0010_0000)
            .await
            .expect("body");
        (status, String::from_utf8(bytes.to_vec()).expect("utf8"))
    }

    pub(super) fn seed_row(&self, prompt_id: &str, version: i64, environment: &str) {
        self.seed_row_with(prompt_id, version, environment, "old");
    }

    pub(super) fn seed_row_with(
        &self,
        prompt_id: &str,
        version: i64,
        environment: &str,
        content: &str,
    ) {
        let mut rows = self.rows.lock().expect("rows");
        let id = format!("seed-{}", rows.len().saturating_add(1));
        rows.push(PromptRow {
            id,
            prompt_id: prompt_id.to_owned(),
            version,
            environment: environment.to_owned(),
            created_by: Some("seeder".to_owned()),
            created_at: fixed_time(),
            updated_at: fixed_time(),
            litellm_params: PromptRowJson::Text(params_json(prompt_id, content)),
            prompt_info: Some(PromptRowJson::Text(info_json("db", environment))),
        });
    }

    pub(super) async fn seed_registry(
        &self,
        prompt_id: &str,
        version: i64,
        environment: &str,
        prompt_type: &str,
    ) {
        let spec_json = format!(
            r#"{{"prompt_id":"{prompt_id}.v{version}","litellm_params":{},"prompt_info":{},"environment":"{environment}","version":{version}}}"#,
            params_json(prompt_id, "old"),
            info_json(prompt_type, environment)
        );
        let spec: PromptSpec = serde_json::from_str(&spec_json).expect("spec");
        let mut registry = self.registry.lock().await;
        let mut callbacks = self.callbacks.lock().await;
        registry
            .initialize_prompt(&spec, None, &mut *callbacks)
            .expect("registered");
        drop(callbacks);
        drop(registry);
        self.calls.lock().expect("calls").clear();
    }
}

fn params_json(prompt_id: &str, content: &str) -> String {
    format!(
        r#"{{"prompt_id":"{prompt_id}","prompt_integration":"dotprompt","dotprompt_content":"{content}"}}"#
    )
}

fn info_json(prompt_type: &str, environment: &str) -> String {
    format!(r#"{{"prompt_type":"{prompt_type}","environment":"{environment}"}}"#)
}
