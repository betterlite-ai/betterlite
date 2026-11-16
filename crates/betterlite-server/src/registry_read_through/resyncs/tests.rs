#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::model::ModelFields;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRow;
use chrono::NaiveDateTime;
use rstest::rstest;

use crate::registry_read_through::RegistryReadThrough;
use crate::registry_read_through::Resync;
use crate::registry_read_through::ResyncError;
use crate::registry_read_through::resyncs::{
    AgentResync, AgentSync, DbObjects, GuardrailResync, GuardrailSync, ModelKey, ModelResync,
    ModelRow, ModelSync, ObjectType, RegistryReadThroughs,
};
use crate::registry_read_through::tests::support::CountingResync;

// (a client is configured, store_model_in_db is true, should_load answers true)
struct FakeDb {
    asks: Arc<Mutex<Vec<ObjectType>>>,
    flags: (bool, bool, bool),
}

impl DbObjects for FakeDb {
    fn has_client(&self) -> bool {
        self.flags.0
    }

    fn store_model_in_db(&self) -> bool {
        self.flags.1
    }

    fn should_load(&self, object_type: ObjectType) -> bool {
        self.asks.lock().unwrap().push(object_type);
        self.flags.2
    }
}

fn enabled_db() -> FakeDb {
    FakeDb {
        asks: Arc::new(Mutex::new(Vec::new())),
        flags: (true, true, true),
    }
}

#[rstest]
#[case::all_three((true, true, true), true)]
#[case::no_client((false, true, true), false)]
#[case::models_not_stored((true, false, true), false)]
#[case::object_type_not_loaded((true, true, false), false)]
fn db_backed_registries_need_a_client_stored_models_and_the_should_load_rule(
    #[case] flags: (bool, bool, bool),
    #[case] expected: bool,
) {
    let db = FakeDb {
        asks: Arc::new(Mutex::new(Vec::new())),
        flags,
    };
    assert_eq!(db.enabled(ObjectType::Models), expected);
}

#[rstest]
#[case::no_client((false, true))]
#[case::models_not_stored((true, false))]
#[case::neither((false, false))]
fn a_disabled_database_never_asks_the_should_load_rule(#[case] flags: (bool, bool)) {
    let asks = Arc::new(Mutex::new(Vec::new()));
    let db = FakeDb {
        asks: Arc::clone(&asks),
        flags: (flags.0, flags.1, true),
    };
    assert!(!db.enabled(ObjectType::Models));
    assert!(asks.lock().unwrap().is_empty());
}

#[derive(Debug, PartialEq, Eq)]
enum ModelCall {
    FindByName(String),
    FindById(String),
    RouterExists,
    AddDeployments,
    AddRows(usize),
}

struct FakeModelSync {
    calls: Arc<Mutex<Vec<ModelCall>>>,
    name_rows: Vec<ModelRow>,
    id_rows: Vec<ModelRow>,
    router: bool,
    deployments: bool,
    failure: Option<&'static str>,
}

impl FakeModelSync {
    fn rows(&self, rows: &[ModelRow]) -> Result<Vec<ModelRow>, ResyncError> {
        if let Some(message) = self.failure {
            return Err(ResyncError::Failed {
                message: message.to_owned(),
            });
        }
        Ok(rows.to_vec())
    }
}

impl ModelSync for FakeModelSync {
    async fn find_rows(&self, by: ModelKey<'_>) -> Result<Vec<ModelRow>, ResyncError> {
        match by {
            ModelKey::Name(name) => {
                self.calls
                    .lock()
                    .unwrap()
                    .push(ModelCall::FindByName(name.to_owned()));
                self.rows(&self.name_rows)
            }
            ModelKey::Id(id) => {
                self.calls
                    .lock()
                    .unwrap()
                    .push(ModelCall::FindById(id.to_owned()));
                self.rows(&self.id_rows)
            }
        }
    }

    async fn router_exists(&self) -> bool {
        self.calls.lock().unwrap().push(ModelCall::RouterExists);
        self.router
    }

    async fn add_deployments_from_db(&self) -> Result<bool, ResyncError> {
        self.calls.lock().unwrap().push(ModelCall::AddDeployments);
        Ok(self.deployments)
    }

    async fn add_rows_to_router(&self, rows: Vec<ModelRow>) -> Result<(), ResyncError> {
        self.calls
            .lock()
            .unwrap()
            .push(ModelCall::AddRows(rows.len()));
        Ok(())
    }
}

fn model_row(model_id: &str, model_name: &str) -> ModelRow {
    ModelRow {
        fields: ModelFields {
            model_id: model_id.to_owned(),
            model_name: model_name.to_owned(),
            litellm_params: Payload(BTreeMap::new()),
            model_info: None,
            blocked: false,
            created_at: None,
            created_by: None,
            updated_at: None,
            updated_by: None,
        },
    }
}

#[tokio::test]
async fn a_disabled_model_resync_returns_false_without_reading_the_database() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let asks = Arc::new(Mutex::new(Vec::new()));
    let db = FakeDb {
        asks: Arc::clone(&asks),
        flags: (true, true, false),
    };
    let sync = FakeModelSync {
        calls: Arc::clone(&calls),
        name_rows: vec![model_row("model-1", "gpt-4o")],
        id_rows: Vec::new(),
        router: true,
        deployments: false,
        failure: None,
    };
    let resync = ModelResync { sync, db };
    assert!(!resync.resync("gpt-4o").await.unwrap());
    assert_eq!(*asks.lock().unwrap(), [ObjectType::Models]);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_model_resync_adds_rows_found_by_name_to_the_router() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeModelSync {
        calls: Arc::clone(&calls),
        name_rows: vec![model_row("model-1", "gpt-4o")],
        id_rows: Vec::new(),
        router: true,
        deployments: false,
        failure: None,
    };
    let resync = ModelResync {
        sync,
        db: enabled_db(),
    };
    assert!(resync.resync("gpt-4o").await.unwrap());
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ModelCall::FindByName("gpt-4o".to_owned()),
            ModelCall::RouterExists,
            ModelCall::AddRows(1),
        ]
    );
}

#[tokio::test]
async fn the_model_resync_falls_back_to_the_model_id() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeModelSync {
        calls: Arc::clone(&calls),
        name_rows: Vec::new(),
        id_rows: vec![model_row("gpt-4o-id", "prod-model")],
        router: true,
        deployments: false,
        failure: None,
    };
    let resync = ModelResync {
        sync,
        db: enabled_db(),
    };
    assert!(resync.resync("gpt-4o-id").await.unwrap());
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ModelCall::FindByName("gpt-4o-id".to_owned()),
            ModelCall::FindById("gpt-4o-id".to_owned()),
            ModelCall::RouterExists,
            ModelCall::AddRows(1),
        ]
    );
}

#[tokio::test]
async fn the_model_resync_returns_false_when_neither_lookup_finds_a_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeModelSync {
        calls: Arc::clone(&calls),
        name_rows: Vec::new(),
        id_rows: Vec::new(),
        router: true,
        deployments: true,
        failure: None,
    };
    let resync = ModelResync {
        sync,
        db: enabled_db(),
    };
    assert!(!resync.resync("ghost-model").await.unwrap());
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ModelCall::FindByName("ghost-model".to_owned()),
            ModelCall::FindById("ghost-model".to_owned()),
        ]
    );
}

#[rstest]
#[case::a_router_appears((true, true))]
#[case::no_router_appears((false, false))]
#[tokio::test]
async fn the_model_resync_returns_whether_building_the_router_found_one(
    #[case] answers: (bool, bool),
) {
    let (deployments, expected) = answers;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeModelSync {
        calls: Arc::clone(&calls),
        name_rows: vec![model_row("model-1", "gpt-4o")],
        id_rows: Vec::new(),
        router: false,
        deployments,
        failure: None,
    };
    let resync = ModelResync {
        sync,
        db: enabled_db(),
    };
    assert_eq!(resync.resync("gpt-4o").await.unwrap(), expected);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ModelCall::FindByName("gpt-4o".to_owned()),
            ModelCall::RouterExists,
            ModelCall::AddDeployments,
        ]
    );
}

#[tokio::test]
async fn a_model_lookup_error_fails_the_resync() {
    let sync = FakeModelSync {
        calls: Arc::new(Mutex::new(Vec::new())),
        name_rows: Vec::new(),
        id_rows: Vec::new(),
        router: true,
        deployments: false,
        failure: Some("the model table is gone"),
    };
    let resync = ModelResync {
        sync,
        db: enabled_db(),
    };
    let error = resync.resync("gpt-4o").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the registry resync failed: the model table is gone"
    );
}

fn naive(text: &str) -> Timestamp {
    Timestamp::Naive(NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").unwrap())
}

fn guardrail_row(guardrail_name: &str) -> GuardrailRow {
    GuardrailRow {
        guardrail_id: "guardrail-1".to_owned(),
        guardrail_name: guardrail_name.to_owned(),
        litellm_params: None,
        guardrail_info: None,
        team_id: None,
        status: "active".to_owned(),
        created_at: naive("2024-01-01T00:00:00"),
        updated_at: naive("2024-01-01T00:00:00"),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum GuardrailCall {
    FindActive(String),
    LoadRow(String),
}

struct FakeGuardrailSync {
    calls: Arc<Mutex<Vec<GuardrailCall>>>,
    row: Option<GuardrailRow>,
    loads_row: bool,
}

impl GuardrailSync for FakeGuardrailSync {
    async fn find_active(&self, name: &str) -> Result<Option<GuardrailRow>, ResyncError> {
        self.calls
            .lock()
            .unwrap()
            .push(GuardrailCall::FindActive(name.to_owned()));
        Ok(self.row.clone())
    }

    async fn load_row(&self, row: GuardrailRow) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push(GuardrailCall::LoadRow(row.guardrail_name));
        self.loads_row
    }
}

#[tokio::test]
async fn a_disabled_guardrail_resync_returns_false_without_asking_for_the_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let asks = Arc::new(Mutex::new(Vec::new()));
    let db = FakeDb {
        asks: Arc::clone(&asks),
        flags: (true, true, false),
    };
    let sync = FakeGuardrailSync {
        calls: Arc::clone(&calls),
        row: Some(guardrail_row("pii")),
        loads_row: true,
    };
    let resync = GuardrailResync { sync, db };
    assert!(!resync.resync("pii").await.unwrap());
    assert_eq!(*asks.lock().unwrap(), [ObjectType::Guardrails]);
    assert!(calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::registered(true)]
#[case::not_registered(false)]
#[tokio::test]
async fn the_guardrail_resync_returns_the_lookup_answer_after_loading_the_row(
    #[case] loads_row: bool,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeGuardrailSync {
        calls: Arc::clone(&calls),
        row: Some(guardrail_row("pii")),
        loads_row,
    };
    let resync = GuardrailResync {
        sync,
        db: enabled_db(),
    };
    assert_eq!(resync.resync("pii").await.unwrap(), loads_row);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            GuardrailCall::FindActive("pii".to_owned()),
            GuardrailCall::LoadRow("pii".to_owned()),
        ]
    );
}

#[tokio::test]
async fn the_guardrail_resync_returns_false_when_no_active_row_exists() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeGuardrailSync {
        calls: Arc::clone(&calls),
        row: None,
        loads_row: true,
    };
    let resync = GuardrailResync {
        sync,
        db: enabled_db(),
    };
    assert!(!resync.resync("pii").await.unwrap());
    assert_eq!(
        *calls.lock().unwrap(),
        [GuardrailCall::FindActive("pii".to_owned())]
    );
}

#[derive(Debug, PartialEq, Eq)]
enum AgentCall {
    Cached(String),
    LoadLocked(String),
}

struct FakeAgentSync {
    calls: Arc<Mutex<Vec<AgentCall>>>,
    cached: bool,
    load_result: bool,
}

impl AgentSync for FakeAgentSync {
    async fn cached(&self, key: &str) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push(AgentCall::Cached(key.to_owned()));
        self.cached
    }

    async fn load_locked(&self, key: &str) -> Result<bool, ResyncError> {
        self.calls
            .lock()
            .unwrap()
            .push(AgentCall::LoadLocked(key.to_owned()));
        Ok(self.load_result)
    }
}

#[tokio::test]
async fn a_disabled_agent_resync_returns_false_without_looking_at_the_registry() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let asks = Arc::new(Mutex::new(Vec::new()));
    let db = FakeDb {
        asks: Arc::clone(&asks),
        flags: (true, true, false),
    };
    let sync = FakeAgentSync {
        calls: Arc::clone(&calls),
        cached: true,
        load_result: true,
    };
    let resync = AgentResync { sync, db };
    assert!(!resync.resync("support-agent").await.unwrap());
    assert_eq!(*asks.lock().unwrap(), [ObjectType::Agents]);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_agent_resync_answers_from_the_local_registry_without_loading() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeAgentSync {
        calls: Arc::clone(&calls),
        cached: true,
        load_result: false,
    };
    let resync = AgentResync {
        sync,
        db: enabled_db(),
    };
    assert!(resync.resync("support-agent").await.unwrap());
    assert_eq!(
        *calls.lock().unwrap(),
        [AgentCall::Cached("support-agent".to_owned())]
    );
}

#[rstest]
#[case::registered(true)]
#[case::not_registered(false)]
#[tokio::test]
async fn the_agent_resync_loads_under_the_lock_when_the_cache_misses(#[case] load_result: bool) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sync = FakeAgentSync {
        calls: Arc::clone(&calls),
        cached: false,
        load_result,
    };
    let resync = AgentResync {
        sync,
        db: enabled_db(),
    };
    assert_eq!(resync.resync("support-agent").await.unwrap(), load_result);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            AgentCall::Cached("support-agent".to_owned()),
            AgentCall::LoadLocked("support-agent".to_owned()),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_three_read_throughs_keep_separate_negative_caches() {
    let model_calls = Arc::new(AtomicUsize::new(0));
    let guardrail_calls = Arc::new(AtomicUsize::new(0));
    let agent_calls = Arc::new(AtomicUsize::new(0));
    let registries = RegistryReadThroughs {
        models: RegistryReadThrough::new(CountingResync {
            calls: Arc::clone(&model_calls),
            found: false,
        }),
        guardrails: RegistryReadThrough::new(CountingResync {
            calls: Arc::clone(&guardrail_calls),
            found: false,
        }),
        agents: RegistryReadThrough::new(CountingResync {
            calls: Arc::clone(&agent_calls),
            found: false,
        }),
    };
    assert!(!registries.models.attempt("gpt-4o").await);
    assert!(!registries.models.attempt("gpt-4o").await);
    assert!(!registries.guardrails.attempt("gpt-4o").await);
    assert_eq!(model_calls.load(Ordering::SeqCst), 1);
    assert_eq!(guardrail_calls.load(Ordering::SeqCst), 1);
    assert_eq!(agent_calls.load(Ordering::SeqCst), 0);
}
