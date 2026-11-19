use axum::Json;
use axum::extract::{Extension, Query, State};
use betterlite_roi::records::{ROIRepositoriesResponse, ROIRepository};
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::roi_github::repositories::repositories;
use crate::roi_github::transport::Transport;
use crate::roi_settings::store::load_settings;

use super::super::access::{READ_FORBIDDEN, READ_ROLES, require_admin};
use super::super::error::{QueryIssue, RoiConnectionsError};
use super::super::state::{RoiConnectionsState, github_client, require_repository};

pub const REPOSITORIES_PATH: &str = "/roi-calculator/repositories";

const MAX_QUERY_CHARS: usize = 200;
const TOO_LONG: &str = "String should have at most 200 characters";
const INT_PARSING: &str = "Input should be a valid integer, unable to parse string as an integer";
const GREATER_EQUAL: &str = "Input should be greater than or equal to 1";
const LESS_EQUAL: &str = "Input should be less than or equal to 1000";

#[derive(Deserialize)]
pub(super) struct RawQuery {
    pub(super) query: Option<String>,
    pub(super) page: Option<String>,
}

pub(super) async fn list_repositories<T>(
    State(state): State<RoiConnectionsState<T>>,
    caller: Option<Extension<AuthContext>>,
    Query(raw): Query<RawQuery>,
) -> Result<Json<ROIRepositoriesResponse>, RoiConnectionsError>
where
    T: Transport + Clone + Send + Sync + 'static,
{
    let context = caller.as_ref().map(|extension| &extension.0);
    require_admin(context, READ_ROLES, READ_FORBIDDEN)?;
    let repository = require_repository(&state)?;
    let (query, page) = parse_query(&raw)?;
    let settings = load_settings(repository, state.cipher.as_ref()).await?;
    let github = github_client(&settings, &state)?;
    let listed = repositories(&github, &query, page).await;
    github.close();
    let (found, has_more) = listed?;
    Ok(Json(ROIRepositoriesResponse {
        repositories: found
            .into_iter()
            .map(|entry| ROIRepository {
                name: entry.full_name,
                visibility: entry.visibility,
                archived: entry.archived,
            })
            .collect(),
        page,
        has_more,
    }))
}

fn parse_query(raw: &RawQuery) -> Result<(String, i64), RoiConnectionsError> {
    let query = raw.query.clone().unwrap_or_default();
    let mut issues = Vec::new();
    if query.chars().count() > MAX_QUERY_CHARS {
        issues.push(issue("string_too_long", "query", TOO_LONG));
    }
    let mut page = 1;
    if let Some(text) = raw.page.as_deref() {
        match parse_page(text) {
            Ok(parsed) => page = parsed,
            Err(found) => issues.push(found),
        }
    }
    if issues.is_empty() {
        return Ok((query, page));
    }
    Err(RoiConnectionsError::Query(issues))
}

fn parse_page(text: &str) -> Result<i64, QueryIssue> {
    let Ok(page) = text.trim().parse::<i64>() else {
        return Err(issue("int_parsing", "page", INT_PARSING));
    };
    if page < 1 {
        return Err(issue("greater_than_equal", "page", GREATER_EQUAL));
    }
    if page > 1000 {
        return Err(issue("less_than_equal", "page", LESS_EQUAL));
    }
    Ok(page)
}

fn issue(kind: &'static str, field: &'static str, msg: &'static str) -> QueryIssue {
    QueryIssue {
        kind,
        loc: vec!["query", field],
        msg: msg.to_owned(),
    }
}

#[path = "repositories/tests/auth.rs"]
mod auth;
#[path = "repositories/tests/errors.rs"]
mod errors;
#[path = "repositories/tests/pagination.rs"]
mod pagination;
#[path = "repositories/tests/query.rs"]
mod query;
