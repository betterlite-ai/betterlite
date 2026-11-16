use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use betterlite_config::sections::content_filter_catalog::categories::{
    category_dirs, find_category_file,
};
use betterlite_config::sections::content_filter_catalog::roots::CATEGORIES_DIR;
use serde::Serialize;

use crate::assets::paths::{is_within, safe_join};
use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::error::CatalogApplyError;

const CATEGORY_PATH: &str = "/guardrails/ui/category_yaml/{category_name}";

#[derive(Clone)]
struct CategoryState {
    roots: Arc<[PathBuf]>,
}

#[derive(Serialize)]
struct CategoryBody {
    category_name: String,
    yaml_content: String,
    file_type: &'static str,
}

pub(super) fn routes(roots: Arc<[PathBuf]>) -> Router {
    Router::new()
        .route(CATEGORY_PATH, get(category_yaml))
        .with_state(CategoryState { roots })
}

async fn category_yaml(
    State(state): State<CategoryState>,
    caller: Option<Extension<AuthContext>>,
    UrlPath(category_name): UrlPath<String>,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let yaml_name = format!("{category_name}.yaml");
    if safe_join(Path::new(CATEGORIES_DIR), &[yaml_name.as_str()]).is_err() {
        return CatalogApplyError::InvalidCategoryName.into_response();
    }
    let roots: Vec<&Path> = state.roots.iter().map(PathBuf::as_path).collect();
    let Some(found) = find_category_file(&category_name, &roots) else {
        return CatalogApplyError::CategoryNotFound { category_name }.into_response();
    };
    if !category_dirs(&roots)
        .iter()
        .any(|category_dir| is_within(&found, category_dir))
    {
        return CatalogApplyError::InvalidCategoryName.into_response();
    }
    read_category(&found, category_name)
}

fn read_category(found: &Path, category_name: String) -> Response {
    let file_type = if found.to_str().is_some_and(|path| path.ends_with(".yaml")) {
        "yaml"
    } else {
        "json"
    };
    match fs::read_to_string(found) {
        Ok(content) => (
            StatusCode::OK,
            Json(CategoryBody {
                category_name,
                yaml_content: content,
                file_type,
            }),
        )
            .into_response(),
        Err(error) => CatalogApplyError::CategoryRead {
            message: error.to_string(),
        }
        .into_response(),
    }
}
