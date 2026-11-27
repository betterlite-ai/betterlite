mod cache;
mod canonical;
mod database;
mod input;
mod world;

use std::sync::{Arc, Mutex, PoisonError};

use self::cache::LoggingCache;
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
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::api::vector_management_read::state::VectorManagementReadState;
use betterlite_server::api::vector_management_write::cache::{RegistryCache, VectorStoreCache};
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

// one call: a fresh database and registry seeded from the case, the one request, then their state
async fn answer(call: &Call) -> HelperResult {
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
    let state = write_state(
        call,
        database.clone(),
        registry.clone(),
        Arc::clone(&events),
    );
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
    let database_rows = match &database {
        Some(database) => json!(database.rows()),
        None => Value::Null,
    };
    let cache_rows = match &registry {
        Some(registry) => json!(stores_of(registry)),
        None => Value::Null,
    };
    let writes = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(canonical(json!({
        "status": status,
        "body": body,
        "database": database_rows,
        "cache": cache_rows,
        "writes": writes,
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
    events: Arc<Mutex<Vec<String>>>,
) -> Arc<VectorManagementWriteState<FakeDatabase>> {
    let cache = registry.as_ref().map(|registry| {
        let cache: Arc<dyn VectorStoreCache> = Arc::new(LoggingCache {
            inner: RegistryCache(Arc::clone(registry)),
            events: Arc::clone(&events),
        });
        cache
    });
    let world = Arc::new(World);
    let reads = Arc::new(VectorManagementReadState {
        database,
        registry,
        permissions: Arc::clone(&world),
        teams: Arc::clone(&world),
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
    Arc::new(VectorManagementWriteState { reads, cache })
}

fn stores_of(registry: &Arc<Mutex<VectorStoreRegistry>>) -> Vec<LiteLlmManagedVectorStore> {
    registry
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .stores
        .clone()
}
