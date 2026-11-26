pub mod caller;
pub mod database;
pub mod error;
pub mod info;
pub mod list;
pub mod query;
pub mod state;

mod tests;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::Uri;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use betterlite_core::api::vector_store_registry::repository::VectorStoreDatabase;
use betterlite_openai::api::vector_store_records::records::pydantic::VectorStoreInfoRequest;

use self::caller::CallerView;
use self::error::ManagementReadError;
use self::info::get_vector_store_info;
use self::list::list_vector_stores;
use self::state::VectorManagementReadState;
use crate::api::vector_management_boundaries::rows::VectorStoreRows;
use crate::auth::auth_identity::AuthContext;
use crate::search_tool_routes::body::decode_body;

pub const LIST_PATH: &str = "/vector_store/list";
pub const V1_LIST_PATH: &str = "/v1/vector_store/list";
pub const INFO_PATH: &str = "/vector_store/info";

// the two list aliases and info, unmounted: the auth stage and the app's router own the mount
pub fn routes<Database>(state: Arc<VectorManagementReadState<Database>>) -> Router
where
    Database: VectorStoreDatabase + VectorStoreRows + Send + Sync + 'static,
{
    Router::new()
        .route(LIST_PATH, get(read_handler::<Database>))
        .route(V1_LIST_PATH, get(read_handler::<Database>))
        .route(INFO_PATH, post(read_handler::<Database>))
        .with_state(state)
}

// the three read routes share one handler: the caller once, then the operation the path names
async fn read_handler<Database>(
    State(state): State<Arc<VectorManagementReadState<Database>>>,
    Extension(context): Extension<AuthContext>,
    uri: Uri,
    body: Bytes,
) -> Result<Response, ManagementReadError>
where
    Database: VectorStoreDatabase + VectorStoreRows + Send + Sync + 'static,
{
    let caller = CallerView::from_principal(&context.principal)?;
    if uri.path() == INFO_PATH {
        let request: VectorStoreInfoRequest = decode_body(&body)?;
        let info = get_vector_store_info(state.as_ref(), &caller, &request).await?;
        return Ok(Json(info).into_response());
    }
    let (page, page_size) = query::pagination(&uri)?;
    let list = list_vector_stores(state.as_ref(), &caller, page, page_size).await?;
    Ok(Json(list).into_response())
}
