pub mod body;
pub mod connection;
mod error;
mod list;
pub mod records;
pub mod state;

mod tests;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, MatchedPath, Path, State};
use axum::http::Method;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::search_tools::convert::tool::SearchTool;
use betterlite_db::search_tools::registry::SearchToolRegistry;
use betterlite_db::search_tools::table_client::SearchToolTableClient;

use self::body::decode_body;
use self::connection::{
    ConnectionAnswer, ConnectionBody, ProvidersBody, available_providers, test_connection,
};
use self::error::{RouteError, list_failure, logged};
use self::list::{ListBody, ToolView, masked_view, merged_tools, visible_tools};
use self::records::{DeletedBody, UpdatedBody, WriteBody};
use self::state::{ConfigSearchTools, SearchToolDatabase, SearchToolPorts, SearchToolState};
use crate::auth::auth_identity::AuthContext;
use crate::search_tool_refresh::RouterSearchRefresh;

const UPDATE_FAILED: &str = "Error updating search tool";
const DELETE_FAILED: &str = "Error deleting search tool";
const INFO_FAILED: &str = "Error getting search tool info";

type Shared<Table, Reconnect, Refresh> = State<Arc<SearchToolState<Table, Reconnect, Refresh>>>;
type Caller = Option<Extension<AuthContext>>;
type Target = (Option<Path<String>>, MatchedPath);

// the writes LiteLLM's create, update and delete endpoints make
enum Change {
    Create(SearchTool),
    Update(String, SearchTool),
    Delete(String),
}

// B8673 mounts these routes; the admin-only table for the write routes is B742's
pub fn routes<Table, Reconnect, Refresh>(
    state: SearchToolState<Table, Reconnect, Refresh>,
) -> Router
where
    Table: SearchToolTableClient + 'static,
    Reconnect: DbReconnect + Clone + Send + Sync + 'static,
    Refresh: RouterSearchRefresh + 'static,
{
    // FastAPI tries the id routes too, so PUT /search_tools/list and GET /search_tools/test_connection read those words as ids
    let write = write_tool::<Table, Reconnect, Refresh>;
    let ports = Arc::clone(&state.ports);
    Router::new()
        .route(
            "/search_tools/list",
            get(list_tools::<Table, Reconnect, Refresh>)
                .put(write)
                .delete(write),
        )
        .route("/search_tools", post(write))
        .route(
            "/search_tools/test_connection",
            post(connection_test)
                .with_state(Arc::clone(&ports))
                .get(tool_info::<Table, Reconnect, Refresh>)
                .put(write)
                .delete(write),
        )
        .route(
            "/search_tools/ui/available_providers",
            get(provider_list).with_state(ports),
        )
        .route(
            "/search_tools/{search_tool_id}",
            get(tool_info::<Table, Reconnect, Refresh>)
                .put(write)
                .delete(write),
        )
        .with_state(Arc::new(state))
}

// a fixed path has no capture, so its last segment is the id FastAPI's {search_tool_id} would read
fn tool_id(target: Target) -> String {
    let (path, matched) = target;
    match path {
        Some(Path(search_tool_id)) => search_tool_id,
        None => matched
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned(),
    }
}

// best effort: the row is committed, so a refresh failure must not answer a 500 the client would retry
async fn refresh_router<Refresh>(refresh: &Refresh)
where
    Refresh: RouterSearchRefresh,
{
    if let Err(error) = refresh.refresh().await {
        tracing::error!("Search tool router refresh failed after a management write: {error}");
    }
}

async fn list_tools<Table, Reconnect, Refresh>(
    State(state): Shared<Table, Reconnect, Refresh>,
    caller: Caller,
) -> Result<Json<ListBody>, RouteError>
where
    Table: SearchToolTableClient,
    Reconnect: DbReconnect + Clone + Send + Sync,
    Refresh: RouterSearchRefresh,
{
    let Extension(context) = caller.ok_or(RouteError::Unauthenticated)?;
    let database = state.database.as_ref().ok_or(RouteError::NoDatabase)?;
    let merged = listed_tools(database, state.ports.config.as_ref()).await?;
    let search_tools = visible_tools(merged, &context.principal, state.ports.teams.as_ref())
        .await
        .map_err(list_failure)?;
    Ok(Json(ListBody { search_tools }))
}

// the config tools a database row does not shadow, then every row; an unreadable config is only a debug line
async fn listed_tools<Table, Reconnect>(
    database: &SearchToolDatabase<Table, Reconnect>,
    config: &dyn ConfigSearchTools,
) -> Result<Vec<ToolView>, RouteError>
where
    Table: SearchToolTableClient,
    Reconnect: DbReconnect + Clone + Send + Sync,
{
    let mut reconnect = database.reconnect.clone();
    let stored =
        SearchToolRegistry::get_all_search_tools_from_db(&database.table, Some(&mut reconnect))
            .await
            .map_err(|error| logged("Error getting search tools", error))?;
    let configured = match config.search_tools().await {
        Ok(tools) => tools,
        Err(error) => {
            tracing::debug!("Could not get config-defined search tools: {error}");
            Vec::new()
        }
    };
    merged_tools(configured, stored).map_err(list_failure)
}

// LiteLLM reads the row first, so a missing id answers 404 before any write
async fn existing_tool<Table>(
    registry: &SearchToolRegistry,
    table: &Table,
    search_tool_id: &str,
    context: &str,
) -> Result<SearchTool, RouteError>
where
    Table: SearchToolTableClient,
{
    let existing = registry
        .get_search_tool_by_id_from_db(search_tool_id, table)
        .await
        .map_err(|error| logged(context, error))?;
    existing.ok_or_else(|| RouteError::NotFound {
        search_tool_id: search_tool_id.to_owned(),
    })
}

// the three writes share the database gate and the best-effort router refresh once the row is committed
async fn commit<Table, Reconnect, Refresh>(
    state: &SearchToolState<Table, Reconnect, Refresh>,
    change: Change,
) -> Result<Response, RouteError>
where
    Table: SearchToolTableClient,
    Reconnect: Send + Sync,
    Refresh: RouterSearchRefresh,
{
    let database = state.database.as_ref().ok_or(RouteError::NoDatabase)?;
    let (registry, table) = (&database.registry, &database.table);
    let answer = match change {
        Change::Create(tool) => {
            let created = registry
                .add_search_tool_to_db(tool, table)
                .await
                .map_err(|error| logged("Error adding search tool to db", error))?;
            let name = &created.search_tool_name;
            tracing::debug!("Successfully added search tool '{name}' to database.");
            Json(created).into_response()
        }
        Change::Update(search_tool_id, tool) => {
            existing_tool(registry, table, &search_tool_id, UPDATE_FAILED).await?;
            let row = registry
                .update_search_tool_in_db(&search_tool_id, &tool, table)
                .await
                .map_err(|error| logged(UPDATE_FAILED, error))?;
            let name = &row.search_tool_name;
            tracing::debug!("Successfully updated search tool '{name}' in database.");
            Json(UpdatedBody::from(row)).into_response()
        }
        Change::Delete(search_tool_id) => {
            existing_tool(registry, table, &search_tool_id, DELETE_FAILED).await?;
            let deleted = registry
                .delete_search_tool_from_db(&search_tool_id, table)
                .await
                .map_err(|error| logged(DELETE_FAILED, error))?;
            tracing::debug!("Successfully deleted search tool from database.");
            Json(DeletedBody::from(deleted)).into_response()
        }
    };
    refresh_router(state.refresh.as_ref()).await;
    Ok(answer)
}

// POST creates, PUT updates and DELETE deletes; DELETE sends no body, so it is never decoded
async fn write_tool<Table, Reconnect, Refresh>(
    State(state): Shared<Table, Reconnect, Refresh>,
    caller: Caller,
    target: Target,
    method: Method,
    body: Bytes,
) -> Result<Response, RouteError>
where
    Table: SearchToolTableClient,
    Reconnect: Send + Sync,
    Refresh: RouterSearchRefresh,
{
    if caller.is_none() {
        return Err(RouteError::Unauthenticated);
    }
    if method == Method::DELETE {
        return commit(&state, Change::Delete(tool_id(target))).await;
    }
    let request: WriteBody = decode_body(&body).map_err(RouteError::Body)?;
    let change = if method == Method::POST {
        Change::Create(request.search_tool)
    } else {
        Change::Update(tool_id(target), request.search_tool)
    };
    commit(&state, change).await
}

async fn tool_info<Table, Reconnect, Refresh>(
    State(state): Shared<Table, Reconnect, Refresh>,
    caller: Caller,
    target: Target,
) -> Result<Json<ToolView>, RouteError>
where
    Table: SearchToolTableClient,
    Reconnect: Send + Sync,
    Refresh: RouterSearchRefresh,
{
    if caller.is_none() {
        return Err(RouteError::Unauthenticated);
    }
    let search_tool_id = tool_id(target);
    let database = state.database.as_ref().ok_or(RouteError::NoDatabase)?;
    let (registry, table) = (&database.registry, &database.table);
    let tool = existing_tool(registry, table, &search_tool_id, INFO_FAILED).await?;
    let view = masked_view(tool, false).map_err(|error| logged(INFO_FAILED, error))?;
    Ok(Json(view))
}

async fn connection_test(
    State(ports): State<Arc<SearchToolPorts>>,
    caller: Caller,
    body: Bytes,
) -> Result<Json<ConnectionAnswer>, RouteError> {
    if caller.is_none() {
        return Err(RouteError::Unauthenticated);
    }
    let request: ConnectionBody = decode_body(&body).map_err(RouteError::Body)?;
    Ok(Json(test_connection(ports.search.as_ref(), request).await))
}

async fn provider_list(
    State(ports): State<Arc<SearchToolPorts>>,
    caller: Caller,
) -> Result<Json<ProvidersBody>, RouteError> {
    if caller.is_none() {
        return Err(RouteError::Unauthenticated);
    }
    Ok(Json(available_providers(ports.catalog.as_ref())))
}
