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

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::Uri;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_openai::api::vector_store_records::records::pydantic::{
    VectorStoreDeleteRequest, VectorStoreUpdateRequest,
};
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use self::create::new_vector_store;
pub use self::create::{NewVectorStore, create_vector_store_in_db};
use self::delete::delete_vector_store;
use self::error::ManagementWriteError;
use self::rows::VectorStoreWrites;
use self::state::VectorManagementWriteState;
use self::update::update_vector_store;
use crate::api::vector_management_read::caller::CallerView;
use crate::auth::auth_identity::AuthContext;
use crate::search_tool_routes::body::decode_body;

pub const NEW_PATH: &str = "/vector_store/new";
pub const DELETE_PATH: &str = "/vector_store/delete";
pub const UPDATE_PATH: &str = "/vector_store/update";

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
        return Ok(Json(response).into_response());
    }
    if uri.path() == DELETE_PATH {
        let request: VectorStoreDeleteRequest = decode_body(&body)?;
        let response = delete_vector_store(state.as_ref(), &caller, &request).await?;
        return Ok(Json(response).into_response());
    }
    let request: VectorStoreUpdateRequest = decode_body(&body)?;
    let response = update_vector_store(state.as_ref(), &caller, &request, &body).await?;
    Ok(Json(response).into_response())
}
