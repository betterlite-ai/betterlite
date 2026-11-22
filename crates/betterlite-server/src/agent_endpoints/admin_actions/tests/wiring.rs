use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_auth::agent_policy::AgentResponse as PolicyAgent;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig, AgentKillSwitchMethod};
use chrono::DateTime;
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::support::{ADMIN, Fixture, caller, kill_switch_path, lock, post};
use crate::agent_endpoints::admin_actions::ports::{
    AgentRegistryLookup, DeliveringTrigger, KillSwitchAuditWriter, KillSwitchTrigger,
};
use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture,
    AuditCallbackRegistry, AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{
    AuditEventSink, AuditLogRepository, AuditRepositoryError, SystemAuditClock,
};
use crate::kill_switch_management::client::{
    KillSwitchHttpClient, KillSwitchResponse, KillSwitchSendOptions,
};
use crate::kill_switch_management::request::KillSwitchRequest;

const POLICY_AGENT: &str = r#"{"agent_id":"agent-1","agent_name":"Agent One","agent_card_params":{},"kill_switch":{"url":"https://agent.example/stop","method":"PUT","headers":{"x-trace":"on"},"query_params":{},"body":null,"auth":{"type":"bearer","token":"sk-live"}}}"#;

#[tokio::test]
async fn the_registry_adapter_keeps_the_stored_kill_switch() {
    let fixture = Fixture::new();
    let registry = AgentRegistry::default();
    let agent: PolicyAgent = serde_json::from_str(POLICY_AGENT).unwrap();
    registry.register_agent(agent);
    let mut state = fixture.state();
    state.registry = Arc::new(registry);
    let reply = post(state, &kill_switch_path("agent-1"), &caller(ADMIN)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let fired = fixture.trigger.fired();
    assert_eq!(fired[0].1.url, "https://agent.example/stop");
    assert_eq!(fired[0].1.method, AgentKillSwitchMethod::Put);
    assert_eq!(fired[0].1.headers["x-trace"], "on");
    let Some(AgentKillSwitchAuth::Bearer { token }) = fired[0].1.auth.as_ref() else {
        panic!("the bearer auth survives the serde round trip");
    };
    assert_eq!(token.expose_secret(), "sk-live");
}

#[test]
fn the_registry_adapter_forwards_ids_for_agent() {
    let registry = AgentRegistry::default();
    let agent: PolicyAgent = serde_json::from_str(POLICY_AGENT).unwrap();
    registry.register_agent(agent);
    let ids = AgentRegistryLookup::ids_for_agent(&registry, "agent-1");
    assert!(ids.contains("agent-1"));
    assert_eq!(ids.len(), 1);
}

#[derive(Debug, Error)]
#[error("the client cannot build requests")]
struct ClientFailure;

struct UnbuildableClient;

struct NoResponse;

impl KillSwitchHttpClient for UnbuildableClient {
    type Error = ClientFailure;
    type Request = ();
    type Response = NoResponse;

    fn build_request(
        &self,
        _request: &KillSwitchRequest,
        _timeout: Duration,
    ) -> Result<(), ClientFailure> {
        Err(ClientFailure)
    }

    fn send(
        &self,
        _request: (),
        _options: KillSwitchSendOptions,
    ) -> impl Future<Output = Result<NoResponse, ClientFailure>> + Send {
        ready(Err(ClientFailure))
    }
}

impl KillSwitchResponse for NoResponse {
    type Error = ClientFailure;

    fn status(&self) -> u16 {
        0
    }

    fn next_text_chunk(
        &mut self,
    ) -> impl Future<Output = Option<Result<String, ClientFailure>>> + Send {
        ready(None)
    }

    fn close(&mut self) -> impl Future<Output = Result<(), ClientFailure>> + Send {
        ready(Ok(()))
    }
}

fn switch_config() -> AgentKillSwitchConfig {
    AgentKillSwitchConfig {
        url: String::from("https://agent.example/stop"),
        method: AgentKillSwitchMethod::Post,
        headers: BTreeMap::new(),
        query_params: BTreeMap::new(),
        body: None,
        auth: None,
    }
}

#[tokio::test]
async fn the_delivering_trigger_reports_the_client_failure() {
    let trigger = DeliveringTrigger {
        client: UnbuildableClient,
    };
    let result = trigger.fire("agent-1", &switch_config()).await;
    assert_eq!(result.agent_id, "agent-1");
    assert_eq!(result.url, "https://agent.example/stop");
    assert_eq!(result.status_code, None);
    assert_eq!(
        result.error.as_deref(),
        Some("the client cannot build requests")
    );
}

struct RecordingRepository {
    records: Arc<Mutex<Vec<AuditLogRecord>>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        lock(&self.records).push(record.clone());
        Ok(())
    }
}

struct NoCallbackRegistry;

impl AuditCallbackRegistry for NoCallbackRegistry {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

struct NoSpawner;

impl AuditTaskSpawner for NoSpawner {
    fn spawn(&self, _task: AuditCallbackFuture) {}
}

struct EnabledSettings;

impl AuditSettings for EnabledSettings {
    fn is_audit_logging_enabled(&self, _explicit: Option<bool>) -> bool {
        true
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &[]
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

#[tokio::test]
async fn the_sink_writer_records_the_kill_switch_row() {
    let records: Arc<Mutex<Vec<AuditLogRecord>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = AuditEventSink {
        settings: Arc::new(EnabledSettings),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(NoCallbackRegistry),
            Arc::new(NoSpawner),
        )),
        repository: Arc::new(RecordingRepository {
            records: Arc::clone(&records),
        }),
        clock: Arc::new(SystemAuditClock),
        new_id: Arc::new(|| String::from("audit-1")),
    };
    let record = AuditLogRecord {
        id: String::from("audit-1"),
        updated_at: DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").unwrap(),
        changed_by: Some(String::from("user-1")),
        changed_by_api_key: None,
        action: AuditAction::KillSwitchFired,
        table_name: AuditTable::Agent,
        object_id: String::from("agent-1"),
        before_value: None,
        updated_values: None,
    };
    KillSwitchAuditWriter::write(&sink, record).await;
    let stored = lock(&records);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, "audit-1");
    assert_eq!(stored[0].object_id, "agent-1");
}
