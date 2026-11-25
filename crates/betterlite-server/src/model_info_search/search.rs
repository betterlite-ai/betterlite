use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::ResourceRowsError;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_model_info::discovery::holds;
use tokio::task::{JoinError, JoinSet};

use crate::db_row_decrypt::deployments::decrypt_model_rows;
use crate::db_row_decrypt::error::DbRowDecryptError;
use crate::model_info_scope::text_field;
use crate::model_team_scope::caller_scope::{
    byok_row_outside_caller_teams, get_caller_byok_team_scope,
};
use crate::model_team_scope::lookup::Decryption;
use crate::model_team_scope::reads::TeamScopeReads;
use crate::team_access::caller::TeamCaller;

const SORTED_SEARCH_DB_FETCH_CAP: i64 = 500;

#[derive(Debug, thiserror::Error)]
pub enum SearchDatabaseError {
    #[error("the model search database query failed: {0}")]
    Database(#[source] TableError),
    #[error("a searched model row is not valid JSON")]
    Row(#[source] serde_json::Error),
    #[error("a searched model row is not a deployment row: {0}")]
    Decode(#[source] ResourceRowsError),
}

#[derive(Debug, thiserror::Error)]
enum DecryptTaskError {
    #[error(transparent)]
    Decrypt(#[from] DbRowDecryptError),
    #[error("the decryption task failed: {0}")]
    Task(#[from] JoinError),
    #[error("the decryption task never ran")]
    Missing,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("counting model search rows failed: {0}")]
    Count(#[source] SearchDatabaseError),
    #[error("fetching model search rows failed: {0}")]
    Fetch(#[source] SearchDatabaseError),
    #[error(transparent)]
    Decrypt(#[from] DecryptTaskError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFilter {
    pub search_lower: String,
    pub exact_name: Option<String>,
    pub excluded_ids: BTreeSet<String>,
}

#[async_trait]
pub trait SearchDatabase: Send + Sync {
    async fn count(&self, filter: &ModelFilter) -> Result<i64, SearchDatabaseError>;
    async fn find(
        &self,
        filter: &ModelFilter,
        take: i64,
    ) -> Result<Vec<ProxyModelTable>, SearchDatabaseError>;
}

pub struct SearchSources<'a, Database: ?Sized, Reads: ?Sized> {
    pub database: &'a Database,
    pub decrypt: &'a Decryption,
    pub reads: Option<&'a Reads>,
    pub caller: Option<&'a TeamCaller>,
}

pub struct SearchOptions<'a> {
    pub page: i64,
    pub size: i64,
    pub sort_by: Option<&'a str>,
    pub model_name: Option<&'a str>,
}

impl Default for SearchOptions<'_> {
    fn default() -> Self {
        Self {
            page: 1,
            size: 50,
            sort_by: None,
            model_name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutcome {
    pub models: Vec<JsonObject>,
    pub total_count: Option<i64>,
}

struct RouterMatches {
    models: Vec<JsonObject>,
    db_ids: BTreeSet<String>,
    config_count: i64,
}

struct FetchRequest<'a, 'options> {
    search_lower: &'a str,
    db_ids: &'a BTreeSet<String>,
    router_count: i64,
    options: &'a SearchOptions<'options>,
    allowed: Option<&'a BTreeSet<String>>,
}

// _apply_search_filter_to_models: router-side matching and BYOK scope, then the bounded DB read-through.
pub async fn apply_search_filter<Database, Reads>(
    models: &[JsonObject],
    search: &str,
    sources: Option<&SearchSources<'_, Database, Reads>>,
    options: &SearchOptions<'_>,
) -> SearchOutcome
where
    Database: SearchDatabase + ?Sized,
    Reads: TeamScopeReads + ?Sized,
{
    if search.trim().is_empty() {
        return SearchOutcome {
            models: models.to_vec(),
            total_count: None,
        };
    }
    let search_lower = search.to_lowercase().trim().to_owned();
    let allowed = match sources {
        Some(found) => get_caller_byok_team_scope(found.caller, found.reads).await,
        None => None,
    };
    let RouterMatches {
        mut matched,
        db_ids,
        config_count,
    } = filter_router_models(models, &search_lower, allowed.as_ref());
    let router_count = config_count.saturating_add(i64::try_from(db_ids.len()).unwrap_or(i64::MAX));
    let mut total_count = router_count;
    if let Some(found) = sources {
        if exact_name_can_match(options.model_name, &search_lower) {
            let request = FetchRequest {
                search_lower: &search_lower,
                db_ids: &db_ids,
                router_count,
                options,
                allowed: allowed.as_ref(),
            };
            match fetch_database(found, &request).await {
                Ok((decrypted, db_total)) => {
                    matched.extend(decrypted);
                    total_count = router_count.saturating_add(db_total);
                }
                Err(error) => {
                    tracing::error!("Error querying database models with search: {error}");
                }
            }
        }
    }
    SearchOutcome {
        models: matched,
        total_count: Some(total_count),
    }
}

fn filter_router_models(
    models: &[JsonObject],
    search_lower: &str,
    allowed: Option<&BTreeSet<String>>,
) -> RouterMatches {
    let empty = JsonObject::new();
    let mut matched = Vec::new();
    let mut db_ids = BTreeSet::new();
    let mut config_count: i64 = 0;
    for model in models {
        if !model_matches_search(model, search_lower) {
            continue;
        }
        let info = match model.get("model_info") {
            Some(Json::Object(fields)) => fields,
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                &empty
            }
        };
        if byok_row_outside_caller_teams(info, allowed) {
            continue;
        }
        let is_db_model = info.get("db_model").is_some_and(holds);
        let model_id = text_field(info, "id").filter(|id| !id.is_empty());
        match model_id {
            Some(id) if is_db_model => {
                db_ids.insert(id.to_owned());
            }
            Some(_) | None => {
                config_count = config_count.saturating_add(1);
            }
        }
        matched.push(model.clone());
    }
    RouterMatches {
        models: matched,
        db_ids,
        config_count,
    }
}

fn model_matches_search(model: &JsonObject, search_lower: &str) -> bool {
    let name = text_field(model, "model_name").unwrap_or_default();
    if name.to_lowercase().contains(search_lower) {
        return true;
    }
    let Some(Json::Object(info)) = model.get("model_info") else {
        return false;
    };
    let public = text_field(info, "team_public_model_name").unwrap_or_default();
    public.to_lowercase().contains(search_lower)
}

fn exact_name_can_match(model_name: Option<&str>, search_lower: &str) -> bool {
    model_name.is_none_or(|name| name.to_lowercase().contains(search_lower))
}

async fn fetch_database<Database, Reads>(
    sources: &SearchSources<'_, Database, Reads>,
    request: &FetchRequest<'_, '_>,
) -> Result<(Vec<JsonObject>, i64), SearchError>
where
    Database: SearchDatabase + ?Sized,
    Reads: TeamScopeReads + ?Sized,
{
    let sorted = request
        .options
        .sort_by
        .is_some_and(|field| !field.is_empty());
    let take = if sorted {
        SORTED_SEARCH_DB_FETCH_CAP
    } else {
        request
            .options
            .page
            .saturating_mul(request.options.size)
            .saturating_sub(request.router_count)
            .max(0)
    };
    let filter = ModelFilter {
        search_lower: request.search_lower.to_owned(),
        exact_name: request.options.model_name.map(str::to_owned),
        excluded_ids: request.db_ids.clone(),
    };
    let db_total = sources
        .database
        .count(&filter)
        .await
        .map_err(SearchError::Count)?;
    if take <= 0 {
        return Ok((Vec::new(), db_total));
    }
    let rows = sources
        .database
        .find(&filter, take)
        .await
        .map_err(SearchError::Fetch)?;
    let scoped: Vec<ProxyModelTable> = rows
        .into_iter()
        .filter(|row| !row_outside_caller_teams(row, request.allowed))
        .collect();
    if scoped.is_empty() {
        return Ok((Vec::new(), db_total));
    }
    let decrypted = decrypt_batch(scoped, sources.decrypt.clone()).await?;
    Ok((decrypted, db_total))
}

fn row_outside_caller_teams(row: &ProxyModelTable, allowed: Option<&BTreeSet<String>>) -> bool {
    let Some(payload) = &row.fields.model_info else {
        return false;
    };
    byok_row_outside_caller_teams(&stored_model_info(payload), allowed)
}

fn stored_model_info(payload: &Payload) -> JsonObject {
    payload
        .0
        .iter()
        .map(|(key, value)| (key.clone(), stored_json(value)))
        .collect()
}

fn stored_json(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(stored_json).collect()),
        JsonData::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), stored_json(item)))
                .collect(),
        ),
    }
}

async fn decrypt_batch(
    rows: Vec<ProxyModelTable>,
    request: Decryption,
) -> Result<Vec<JsonObject>, DecryptTaskError> {
    let tasks = request.tasks.clone();
    let task = tasks.spawn(async move {
        let mut blocking = JoinSet::new();
        blocking.spawn_blocking(move || {
            decrypt_model_rows(
                &rows,
                &request.cipher,
                &request.secrets,
                &request.environment,
                &request.warnings,
            )
        });
        match blocking.join_next().await {
            Some(Ok(decrypted)) => decrypted.map_err(DecryptTaskError::from),
            Some(Err(error)) => Err(DecryptTaskError::from(error)),
            None => Err(DecryptTaskError::Missing),
        }
    });
    task.await.map_err(DecryptTaskError::from)?
}
