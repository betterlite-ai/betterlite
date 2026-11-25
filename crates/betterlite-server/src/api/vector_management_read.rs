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
use axum::routing::{get, post};
use axum::{Json, Router};
use betterlite_core::api::vector_store_registry::repository::VectorStoreDatabase;
use betterlite_openai::api::vector_store_records::records::pydantic::VectorStoreInfoRequest;
use betterlite_openai::api::vector_store_records::records::vector::ManagedVectorStoreList;

use self::caller::CallerView;
use self::error::ManagementReadError;
use self::info::{InfoResponse, get_vector_store_info};
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
        .route(LIST_PATH, get(list_handler::<Database>))
        .route(V1_LIST_PATH, get(list_handler::<Database>))
        .route(INFO_PATH, post(info_handler::<Database>))
        .with_state(state)
}

async fn list_handler<Database>(
    State(state): State<Arc<VectorManagementReadState<Database>>>,
    Extension(context): Extension<AuthContext>,
    uri: Uri,
) -> Result<Json<ManagedVectorStoreList>, ManagementReadError>
where
    Database: VectorStoreDatabase + VectorStoreRows + Send + Sync + 'static,
{
    let page = query::integer(&uri, "page", 1)?;
    let page_size = query::integer(&uri, "page_size", 100)?;
    let caller = CallerView::from_principal(&context.principal)?;
    let list = list_vector_stores(state.as_ref(), &caller, page, page_size).await?;
    Ok(Json(list))
}

async fn info_handler<Database>(
    State(state): State<Arc<VectorManagementReadState<Database>>>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> Result<Json<InfoResponse>, ManagementReadError>
where
    Database: VectorStoreDatabase + VectorStoreRows + Send + Sync + 'static,
{
    let request: VectorStoreInfoRequest = decode_body(&body)?;
    let caller = CallerView::from_principal(&context.principal)?;
    let response = get_vector_store_info(state.as_ref(), &caller, &request).await?;
    Ok(Json(response))
}
