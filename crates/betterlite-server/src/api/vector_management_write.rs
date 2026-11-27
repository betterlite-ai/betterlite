pub mod cache;
pub mod create;
pub mod delete;
pub mod error;
pub mod params;
pub mod response;
pub mod rows;
pub mod state;
pub mod update;

mod tests;

use std::sync::Arc;

use axum::Json as AxumJson;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::Uri;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::{
    VectorStoreDeleteRequest, VectorStoreUpdateRequest,
};
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use self::cache::VectorStoreCache;
use self::create::{new_vector_store, optional_metadata_text, params_text};
use self::delete::delete_vector_store;
use self::error::ManagementWriteError;
use self::rows::{NewVectorStoreRecord, VectorStoreWrites};
use self::state::VectorManagementWriteState;
use self::update::update_vector_store;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_boundaries::rows::VectorStoreRows;
use crate::api::vector_management_read::caller::CallerView;
use crate::auth::auth_identity::AuthContext;
use crate::search_tool_routes::body::decode_body;

pub const NEW_PATH: &str = "/vector_store/new";
pub const DELETE_PATH: &str = "/vector_store/delete";
pub const UPDATE_PATH: &str = "/vector_store/update";

// create_vector_store_in_db's arguments, so B3464 passes the same nine values the endpoint passes
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewVectorStore {
    pub vector_store_id: String,
    pub custom_llm_provider: String,
    pub vector_store_name: Option<String>,
    pub vector_store_description: Option<String>,
    pub vector_store_metadata: Option<Json>,
    pub litellm_params: Option<JsonObject>,
    pub litellm_credential_name: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
}

// create_vector_store_in_db: duplicate check, insert, registry add, and no rollback if the add fails
pub async fn create_vector_store_in_db<Database>(
    database: Option<&Database>,
    cache: Option<&dyn VectorStoreCache>,
    vector_store: NewVectorStore,
) -> Result<LiteLlmManagedVectorStore, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    let Some(database) = database else {
        return Err(ManagementWriteError::DatabaseNotConnected);
    };
    let vector_store_id = vector_store.vector_store_id.clone();
    let existing = VectorStoreRows::find_unique(database, &vector_store_id)
        .await
        .map_err(VectorManagementError::Rows)?;
    if existing.is_some() {
        return Err(ManagementWriteError::AlreadyExists { vector_store_id });
    }
    let record = NewVectorStoreRecord {
        vector_store_id: vector_store_id.clone(),
        custom_llm_provider: vector_store.custom_llm_provider,
        vector_store_name: vector_store.vector_store_name,
        vector_store_description: vector_store.vector_store_description,
        vector_store_metadata: optional_metadata_text(vector_store.vector_store_metadata.as_ref())?,
        litellm_credential_name: vector_store.litellm_credential_name,
        litellm_params: params_text(vector_store.litellm_params.as_ref())?,
        team_id: vector_store.team_id,
        user_id: vector_store.user_id,
    };
    let created = VectorStoreWrites::create(database, record)
        .await
        .map_err(VectorManagementError::Rows)?;
    if let Some(cache) = cache {
        cache.add(created.clone())?;
    }
    tracing::info!("Vector store {vector_store_id} created in database successfully");
    Ok(created)
}

// the three management writes, unmounted: the auth stage and the app's router own the mount
pub fn routes<Database>(state: Arc<VectorManagementWriteState<Database>>) -> Router
where
    Database: VectorStoreWrites + Send + Sync + 'static,
{
    Router::new()
        .route(NEW_PATH, post(write_handler::<Database>))
        .route(DELETE_PATH, post(write_handler::<Database>))
        .route(UPDATE_PATH, post(write_handler::<Database>))
        .with_state(state)
}

// the three writes share one handler: the caller once, then the operation the path names
async fn write_handler<Database>(
    State(state): State<Arc<VectorManagementWriteState<Database>>>,
    Extension(context): Extension<AuthContext>,
    uri: Uri,
    body: Bytes,
) -> Result<Response, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync + 'static,
{
    let caller = CallerView::from_principal(&context.principal)?;
    if uri.path() == NEW_PATH {
        let request: LiteLlmManagedVectorStore = decode_body(&body)?;
        let response = new_vector_store(state.as_ref(), &caller, &request).await?;
        return Ok(AxumJson(response).into_response());
    }
    if uri.path() == DELETE_PATH {
        let request: VectorStoreDeleteRequest = decode_body(&body)?;
        let response = delete_vector_store(state.as_ref(), &caller, &request).await?;
        return Ok(AxumJson(response).into_response());
    }
    let request: VectorStoreUpdateRequest = decode_body(&body)?;
    let response = update_vector_store(state.as_ref(), &caller, &request, &body).await?;
    Ok(AxumJson(response).into_response())
}
