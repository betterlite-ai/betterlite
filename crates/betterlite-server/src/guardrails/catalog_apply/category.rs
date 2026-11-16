use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use betterlite_config::sections::content_filter_catalog::categories::{
    category_dirs, find_category_file,
};
use betterlite_config::sections::content_filter_catalog::roots::CATEGORIES_DIR;
use serde::Serialize;
use tokio_util::task::TaskTracker;

use crate::assets::paths::{is_within, safe_join};
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::error::CatalogApplyError;

const CATEGORY_PATH: &str = "/guardrails/ui/category_yaml/{category_name}";

#[derive(Clone)]
struct CategoryState {
    roots: Arc<[PathBuf]>,
    tasks: TaskTracker,
}

#[derive(Serialize)]
struct CategoryBody {
    category_name: String,
    yaml_content: String,
    file_type: &'static str,
}

pub(super) fn routes(roots: Arc<[PathBuf]>, tasks: TaskTracker) -> Router {
    Router::new()
        .route(CATEGORY_PATH, get(category_yaml))
        .with_state(CategoryState { roots, tasks })
}

async fn category_yaml(
    State(state): State<CategoryState>,
    caller: Option<Extension<AuthContext>>,
    UrlPath(category_name): UrlPath<String>,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let roots = Arc::clone(&state.roots);
    let joined = state
        .tasks
        .spawn_blocking(move || read_category(&roots, category_name))
        .await;
    let read = match joined {
        Ok(read) => read,
        Err(source) => Err(CatalogApplyError::CategoryBlocking { source }),
    };
    match read {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(error) => error.into_response(),
    }
}

// LiteLLM's four steps are blocking calls, so they run together on the blocking pool
fn read_category(
    roots: &[PathBuf],
    category_name: String,
) -> Result<CategoryBody, CatalogApplyError> {
    let yaml_name = format!("{category_name}.yaml");
    if safe_join(Path::new(CATEGORIES_DIR), &[yaml_name.as_str()]).is_err() {
        return Err(CatalogApplyError::InvalidCategoryName);
    }
    let search_roots: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
    let Some(found) = find_category_file(&category_name, &search_roots) else {
        return Err(CatalogApplyError::CategoryNotFound { category_name });
    };
    if !category_dirs(&search_roots)
        .iter()
        .any(|category_dir| is_within(&found, category_dir))
    {
        return Err(CatalogApplyError::InvalidCategoryName);
    }
    let file_type = if found
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("yaml"))
    {
        "yaml"
    } else {
        "json"
    };
    let yaml_content =
        fs::read_to_string(found).map_err(|source| CatalogApplyError::CategoryRead { source })?;
    Ok(CategoryBody {
        category_name,
        yaml_content,
        file_type,
    })
}
