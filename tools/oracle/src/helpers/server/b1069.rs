mod canonical;
mod database;
mod input;
mod world;

use std::sync::{Arc, Mutex, PoisonError};

use self::canonical::canonical;
use self::database::FakeDatabase;
use self::input::{Call, Route};
use self::world::{NoPrivileges, World, auth_context, failure};
use axum::body::{Body, to_bytes};
use axum::http::Request;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::sections::management_rbac::{FeatureSwitches, Settings};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::api::vector_management_read::state::VectorManagementReadState;
use betterlite_server::api::vector_management_write::state::VectorManagementWriteState;
use betterlite_server::api::vector_management_write::{DELETE_PATH, NEW_PATH, UPDATE_PATH, routes};
use chrono::Utc;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tower::ServiceExt as _;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        runtime.block_on(async {
            let mut answers: Vec<Value> = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                answers.push(answer(call).await?);
            }
            Ok(json!(answers))
        })
    }
}

// the per-call state: the event log, the table the writes change, and their registry
struct Fixture {
    events: Arc<Mutex<Vec<String>>>,
    database: Option<FakeDatabase>,
    registry: Option<Arc<Mutex<VectorStoreRegistry>>>,
}

fn fixture(call: &Call) -> Fixture {
    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let database = call
        .setup
        .database
        .clone()
        .map(|rows| FakeDatabase::new(rows, Arc::clone(&events)));
    let registry = call.setup.registry.clone().map(|stores| {
        Arc::new(Mutex::new(VectorStoreRegistry {
            stores,
            clock: Arc::new(Utc::now),
            credentials: Arc::new(CredentialSnapshot::default()),
        }))
    });
    Fixture {
        events,
        database,
        registry,
    }
}

// one call: a fresh database and registry seeded from the case, the one request, then their state
async fn answer(call: &Call) -> HelperResult {
    let Fixture {
        events,
        database,
        registry,
    } = fixture(call);
    let state = write_state(call, database.clone(), registry.clone());
    let request = Request::builder()
        .method("POST")
        .uri(path(call.route))
        .extension(auth_context(&call.setup.caller)?)
        .body(Body::from(
            serde_json::to_vec(&call.body).map_err(|error| failure(error.to_string()))?,
        ))
        .map_err(|error| failure(error.to_string()))?;
    let response = routes(state)
        .oneshot(request)
        .await
        .map_err(|error| failure(error.to_string()))?;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| failure(error.to_string()))?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|error| failure(error.to_string()))?;
    Ok(canonical(json!({
        "status": status,
        "body": body,
        "database": rows_of(database.as_ref()),
        "cache": stores_of(registry.as_deref()),
        "writes": events.lock().unwrap_or_else(PoisonError::into_inner).clone(),
    })))
}

const fn path(route: Route) -> &'static str {
    match route {
        Route::New => NEW_PATH,
        Route::Delete => DELETE_PATH,
        Route::Update => UPDATE_PATH,
    }
}

fn write_state(
    call: &Call,
    database: Option<FakeDatabase>,
    registry: Option<Arc<Mutex<VectorStoreRegistry>>>,
) -> Arc<VectorManagementWriteState<FakeDatabase>> {
    let world = Arc::new(World);
    let permissions = Arc::clone(&world);
    let teams = Arc::clone(&world);
    let reads = Arc::new(VectorManagementReadState {
        database,
        registry,
        permissions,
        teams,
        session_teams: world,
        privileges: Arc::new(NoPrivileges),
        settings: Settings {
            agents: FeatureSwitches::default(),
            vector_stores: FeatureSwitches {
                disabled: call.setup.disabled,
                team_admins_allowed: false,
            },
        },
        limits: ApiLimits::default(),
    });
    Arc::new(VectorManagementWriteState::from(reads))
}

// the rows the table holds after the call, or null when the case seeds no database
fn rows_of(database: Option<&FakeDatabase>) -> Value {
    database.map_or(Value::Null, |table| {
        json!(
            table
                .inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .rows
        )
    })
}

// the stores the registry holds after the call, or null when the case seeds no registry
fn stores_of(registry: Option<&Mutex<VectorStoreRegistry>>) -> Value {
    registry.map_or(Value::Null, |cache| {
        json!(cache.lock().unwrap_or_else(PoisonError::into_inner).stores)
    })
}
