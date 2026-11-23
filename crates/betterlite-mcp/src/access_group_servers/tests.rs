#![cfg(test)]

use std::error::Error as StdError;
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::RwLock;
use tracing::field::{Field as EventField, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use super::{
    AccessGroupServerRows, AccessGroupServersError, config_server_ids_for_access_groups,
    db_server_ids_for_access_groups, servers_from_access_groups,
};
use crate::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use crate::server_record::MCPServer;

#[derive(Debug, thiserror::Error)]
#[error("the fake rows port failed")]
struct PortFailure;

struct PortCall {
    access_groups: Vec<String>,
    use_writer: bool,
}

struct FakeRows {
    server_ids: Vec<String>,
    failing: bool,
    calls: Mutex<Vec<PortCall>>,
}

impl FakeRows {
    fn answering(server_ids: &[&str]) -> Self {
        Self {
            server_ids: groups(server_ids),
            failing: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            server_ids: Vec::new(),
            failing: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<PortCall> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| PortCall {
                access_groups: call.access_groups.clone(),
                use_writer: call.use_writer,
            })
            .collect()
    }
}

impl AccessGroupServerRows for FakeRows {
    type Error = PortFailure;

    fn server_ids_with_any_access_group(
        &self,
        access_groups: &[String],
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send {
        async move {
            self.calls.lock().unwrap().push(PortCall {
                access_groups: access_groups.to_vec(),
                use_writer,
            });
            if self.failing {
                return Err(PortFailure);
            }
            Ok(self.server_ids.clone())
        }
    }
}

#[derive(Clone, Default)]
struct DebugLogs(Arc<Mutex<Vec<String>>>);

impl DebugLogs {
    fn messages(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &EventField, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &EventField, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for DebugLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0.lock().unwrap().push(field.0);
    }
}

fn settings() -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: true,
        internal_ip_ranges: None,
    }
}

fn groups(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn server(server_id: &str, access_groups: Option<&[&str]>) -> MCPServer {
    let listed = access_groups.map_or_else(
        || "null".to_owned(),
        |names| {
            let quoted: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
            format!("[{}]", quoted.join(","))
        },
    );
    let text = format!(
        r#"{{"server_id": "{server_id}", "name": "Name {server_id}", "transport": "http", "access_groups": {listed}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

fn server_registry(config_servers: Vec<MCPServer>, db_servers: Vec<MCPServer>) -> ServerRegistry {
    ServerRegistry::new(settings(), config_servers, db_servers)
}

fn shared(registry: ServerRegistry) -> SharedRegistry {
    Arc::new(RwLock::new(registry))
}

fn capture_debug() -> (DebugLogs, DefaultGuard) {
    let logs = DebugLogs::default();
    let guard = set_default(tracing_subscriber::registry().with(logs.clone()));
    (logs, guard)
}

#[test]
fn config_match_on_one_shared_name_of_several() {
    let registry = server_registry(
        vec![
            server("alpha", Some(&["team-a", "team-b"])),
            server("beta", Some(&["team-b"])),
            server("gamma", Some(&["team-c"])),
        ],
        Vec::new(),
    );
    assert_eq!(
        config_server_ids_for_access_groups(&registry, &groups(&["team-b"])),
        ["alpha", "beta"]
    );
}

#[test]
fn config_no_shared_name_matches_nothing() {
    let registry = server_registry(
        vec![
            server("alpha", Some(&["team-a"])),
            server("beta", Some(&["team-b"])),
        ],
        Vec::new(),
    );
    assert!(config_server_ids_for_access_groups(&registry, &groups(&["team-z"])).is_empty());
}

#[test]
fn config_none_or_empty_groups_never_match() {
    let registry = server_registry(
        vec![server("absent", None), server("empty", Some(&[]))],
        Vec::new(),
    );
    assert!(config_server_ids_for_access_groups(&registry, &groups(&["team-a"])).is_empty());
}

#[test]
fn config_empty_request_matches_nothing() {
    let registry = server_registry(
        vec![
            server("alpha", Some(&["team-a"])),
            server("beta", Some(&["team-b"])),
        ],
        Vec::new(),
    );
    assert!(config_server_ids_for_access_groups(&registry, &[]).is_empty());
}

#[test]
fn config_database_server_group_is_ignored() {
    let registry = server_registry(
        vec![server("config", None)],
        vec![server("database", Some(&["team-a"]))],
    );
    assert!(config_server_ids_for_access_groups(&registry, &groups(&["team-a"])).is_empty());
}

#[test]
fn config_returns_matching_ids_in_config_order() {
    let registry = server_registry(
        vec![
            server("zulu", Some(&["team-a"])),
            server("mike", Some(&["team-b"])),
            server("alpha", Some(&["team-a"])),
        ],
        Vec::new(),
    );
    assert_eq!(
        config_server_ids_for_access_groups(&registry, &groups(&["team-a"])),
        ["zulu", "alpha"]
    );
}

#[tokio::test]
async fn database_empty_request_skips_the_port() {
    let rows = FakeRows::answering(&["s1"]);
    let found = db_server_ids_for_access_groups(Some(&rows), &[], false)
        .await
        .expect("an empty request cannot fail");
    assert!(found.is_empty());
    assert!(rows.calls().is_empty());
}

#[tokio::test]
async fn database_no_rows_skips_the_port() {
    let found = db_server_ids_for_access_groups(None::<&FakeRows>, &groups(&["team-a"]), false)
        .await
        .expect("no rows cannot fail");
    assert!(found.is_empty());
}

#[tokio::test]
async fn database_port_receives_use_writer() {
    let rows = FakeRows::answering(&["s1"]);
    let requested = groups(&["team-a"]);
    let read = db_server_ids_for_access_groups(Some(&rows), &requested, false)
        .await
        .expect("the port answers");
    let fresh = db_server_ids_for_access_groups(Some(&rows), &requested, true)
        .await
        .expect("the port answers");
    assert_eq!(read, ["s1"]);
    assert_eq!(fresh, ["s1"]);
    let calls = rows.calls();
    assert_eq!(calls.len(), 2);
    assert!(!calls[0].use_writer);
    assert!(calls[1].use_writer);
    assert_eq!(calls[0].access_groups, ["team-a"]);
    assert_eq!(calls[1].access_groups, ["team-a"]);
}

#[tokio::test]
async fn database_fault_without_writer_returns_empty_and_logs_debug() {
    let rows = FakeRows::failing();
    let (logs, _guard) = capture_debug();
    let found = db_server_ids_for_access_groups(Some(&rows), &groups(&["team-a"]), false)
        .await
        .expect("a swallowed fault is not an error");
    assert!(found.is_empty());
    let error = PortFailure;
    assert_eq!(
        logs.messages(),
        [format!(
            "Error getting MCP servers from access groups: {error}"
        )]
    );
}

#[tokio::test]
async fn database_fault_with_writer_returns_the_error() {
    let rows = FakeRows::failing();
    let error = db_server_ids_for_access_groups(Some(&rows), &groups(&["team-a"]), true)
        .await
        .expect_err("the fault reaches the caller");
    let source = StdError::source(&error).expect("the port failure is the source");
    assert_eq!(source.to_string(), "the fake rows port failed");
}

#[tokio::test]
async fn combined_config_ids_come_first_then_new_database_ids() {
    let registry = shared(server_registry(
        vec![server("config-a", Some(&["team-a"]))],
        Vec::new(),
    ));
    let rows = FakeRows::answering(&["config-a", "database-b"]);
    let found = servers_from_access_groups(&registry, Some(&rows), &groups(&["team-a"]), false)
        .await
        .expect("the default policy never fails");
    assert_eq!(found, ["config-a", "database-b"]);
}

#[tokio::test]
async fn combined_requires_fresh_policy_reaches_the_port_as_use_writer() {
    let registry = shared(server_registry(
        vec![server("config-a", Some(&["team-a"]))],
        Vec::new(),
    ));
    let rows = FakeRows::answering(&["database-b"]);
    let requested = groups(&["team-a"]);
    let fresh = servers_from_access_groups(&registry, Some(&rows), &requested, true)
        .await
        .expect("the port answers");
    let cached = servers_from_access_groups(&registry, Some(&rows), &requested, false)
        .await
        .expect("the port answers");
    assert_eq!(fresh, ["config-a", "database-b"]);
    assert_eq!(cached, ["config-a", "database-b"]);
    let calls = rows.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].use_writer);
    assert!(!calls[1].use_writer);
}

#[tokio::test]
async fn combined_fresh_policy_fault_is_returned() {
    let registry = shared(server_registry(
        vec![server("config-a", Some(&["team-a"]))],
        Vec::new(),
    ));
    let rows = FakeRows::failing();
    let error = servers_from_access_groups(&registry, Some(&rows), &groups(&["team-a"]), true)
        .await
        .expect_err("the fresh policy propagates the fault");
    assert!(matches!(error, AccessGroupServersError::Rows(_)));
}

#[tokio::test]
async fn combined_fault_without_fresh_policy_keeps_the_config_ids() {
    let registry = shared(server_registry(
        vec![server("config-a", Some(&["team-a"]))],
        Vec::new(),
    ));
    let rows = FakeRows::failing();
    let (logs, _guard) = capture_debug();
    let found = servers_from_access_groups(&registry, Some(&rows), &groups(&["team-a"]), false)
        .await
        .expect("the default policy swallows the fault");
    assert_eq!(found, ["config-a"]);
    assert_eq!(logs.messages().len(), 1);
}
