#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::guardrail_identity::item::{
    CallbackArgs, GuardrailItem, GuardrailValue,
};
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_config::sections::guardrail_params::{RejectedGuardrail, Settings};
use betterlite_config::sections::legacy_guardrails::{
    Raw as LegacyRaw, Settings as LegacySettings,
};
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::error::RegistryError as LoggingRegistryError;
use betterlite_logging::registry::phases::CallbackList;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::task::yield_now;
use tokio::time::timeout;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use super::error::StartupError;
use super::from_config::start_from_config;
use super::legacy::{
    LegacyCallbackStarter, LegacyPlan, plan_legacy_callbacks, start_legacy_guardrails,
};
use super::poll::poll_database;
use super::reconcile::GuardrailReconcile;
use super::router_list::router_guardrail_list;
use super::runtime::GuardrailRuntime;
use super::starter::RegistryStarter;
use crate::guardrail_registry::build::GuardrailFactories;
use crate::guardrail_registry::error::RegistryError;
use crate::guardrail_registry::handler::{HandlerSettings, InMemoryGuardrailHandler};
use crate::guardrail_registry::registry::GuardrailRegistry;
use crate::guardrail_registry::rows::{
    GuardrailRow, GuardrailRows, GuardrailUpdate, NewGuardrailRow, RowsError,
};
use crate::tool_policy::{RequestToolNames, ToolPolicyLookup};

const BEDROCK_PRE: &str = r#"{"guardrail":"bedrock","mode":"pre_call"}"#;
const BEDROCK_POST: &str = r#"{"guardrail":"bedrock","mode":"post_call"}"#;
const UNSUPPORTED: &str = r#"{"guardrail":"no-such-guardrail","mode":"pre_call"}"#;
const HIDE_SECRETS: &str = r#"{"guardrail":"hide-secrets","mode":"pre_call"}"#;

fn guardrail(id: Option<&str>, name: &str, params_json: &str) -> Guardrail {
    Guardrail {
        guardrail_id: id.map(str::to_owned),
        guardrail_name: name.to_owned(),
        litellm_params: decode_litellm_params(params_json.as_bytes()).unwrap(),
        guardrail_info: None,
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    }
}

fn row(id: &str, name: &str, params_json: &str) -> GuardrailRow {
    let params = serde_json::to_string(params_json).unwrap();
    let text = format!(
        r#"{{"guardrail_id":"{id}","guardrail_name":"{name}","litellm_params":{params},"guardrail_info":"{{}}","team_id":null,"status":"active","created_at":"2026-01-02T03:04:05","updated_at":"2026-01-02T03:04:05"}}"#
    );
    serde_json::from_str(&text).unwrap()
}

struct NoPolicies;

impl ToolPolicyLookup for NoPolicies {
    fn is_initialized(&self) -> bool {
        false
    }

    fn effective_policies(
        &self,
        _tool_names: &[String],
        _key_permission_id: Option<&str>,
        _team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn output_policy(&self, _tool_name: &str) -> String {
        String::new()
    }
}

struct NoNames;

impl RequestToolNames for NoNames {
    fn names(&self, _route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

fn handler_with(env: BTreeMap<String, String>) -> InMemoryGuardrailHandler {
    let factories = GuardrailFactories::new(Arc::new(NoPolicies), Arc::new(NoNames));
    let settings = HandlerSettings {
        strict_modes: true,
        global_skip_tool_message: false,
    };
    InMemoryGuardrailHandler::new(factories, Box::new(env), settings)
}

fn handler() -> InMemoryGuardrailHandler {
    handler_with(BTreeMap::new())
}

struct FakeRows {
    active: Mutex<VecDeque<Result<Vec<GuardrailRow>, RowsError>>>,
    reads: Arc<AtomicUsize>,
}

impl GuardrailRows for FakeRows {
    fn create(
        &self,
        _row: NewGuardrailRow,
    ) -> impl Future<Output = Result<GuardrailRow, RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn delete(&self, _guardrail_id: &str) -> impl Future<Output = Result<(), RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn update(
        &self,
        _guardrail_id: &str,
        _row: GuardrailUpdate,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn find_active(&self) -> impl Future<Output = Result<Vec<GuardrailRow>, RowsError>> + Send {
        self.reads.fetch_add(1, Ordering::SeqCst);
        ready(self.active.lock().unwrap().pop_front().expect("scripted"))
    }

    fn find_by_id(
        &self,
        _guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn find_by_name(
        &self,
        _guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }
}

type Reads = Arc<AtomicUsize>;

fn runtime_with(
    handler: InMemoryGuardrailHandler,
    polls: Vec<Result<Vec<GuardrailRow>, RowsError>>,
) -> (GuardrailRuntime<FakeRows>, Reads) {
    let reads = Arc::new(AtomicUsize::new(0));
    let rows = FakeRows {
        active: Mutex::new(VecDeque::from(polls)),
        reads: Arc::clone(&reads),
    };
    let runtime = GuardrailRuntime::new(handler, GuardrailRegistry::new(rows));
    (runtime, reads)
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

fn logged(captured: &Captured) -> String {
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}

fn held_ids(handler: &InMemoryGuardrailHandler) -> Vec<String> {
    handler
        .list_in_memory_guardrails()
        .into_iter()
        .filter_map(|guardrail| guardrail.guardrail_id)
        .collect()
}

async fn runtime_ids(runtime: &GuardrailRuntime<FakeRows>) -> Vec<String> {
    let guard = runtime.handler().read().await;
    held_ids(&guard)
}

include!("tests/reconcile.rs");
include!("tests/sync.rs");
include!("tests/runtime.rs");
include!("tests/from_config.rs");
include!("tests/router_list.rs");
include!("tests/poll.rs");
include!("tests/legacy.rs");
include!("tests/starter.rs");
