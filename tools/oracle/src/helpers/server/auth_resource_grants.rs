use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_resource_grants::managed_rows::{
    ManagedRowsQuery, ManagedVectorStoreCache, ManagedVectorStoreRows, VectorStoreRow,
    VectorStoreRowsFailure, vector_store_rows_by_uuids,
};
use betterlite_auth::auth_resource_grants::search::{
    can_key_call_search_tool, can_team_call_search_tool,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

// LiteLLM's management-object cache default, which every row write carries as its ttl
const MANAGEMENT_TTL_SECONDS: f64 = 60.0;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SearchKey {
        search_tool_name: String,
        key: KeyFields,
    },
    SearchTeam {
        search_tool_name: String,
        team: Option<TeamFields>,
    },
    ManagedRows {
        rows: Vec<RowFixture>,
        rounds: Vec<Vec<String>>,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct KeyFields {
    object_permission_id: Option<String>,
    permission: Option<PermissionFields>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionFields {
    object_permission_id: String,
    #[serde(default)]
    search_tools: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TeamFields {
    team_id: String,
    #[serde(default)]
    object_permission_id: Option<String>,
    #[serde(default)]
    permission: Option<PermissionFields>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowFixture {
    id: String,
    model_dump: BTreeMap<String, JsonData>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SearchKey {
                search_tool_name,
                key,
            } => search_key(search_tool_name, key),
            Input::SearchTeam {
                search_tool_name,
                team,
            } => search_team(search_tool_name, team.as_ref()),
            Input::ManagedRows { rows, rounds } => managed_rows(rows, rounds),
        }
    }
}

fn search_key(search_tool_name: &str, key: &KeyFields) -> HelperResult {
    let mut object = BTreeMap::new();
    object.insert("token", json!("oracle-key"));
    if let Some(id) = &key.object_permission_id {
        object.insert("object_permission_id", json!(id));
    }
    if let Some(permission) = &key.permission {
        object.insert("object_permission", permission_data(permission));
    }
    let payload: BTreeMap<String, JsonData> = serde_json::from_value(json!(object))?;
    let token = UserAPIKeyAuth::construct(AuthInput::Dict(payload)).or_value_error()?;
    match can_key_call_search_tool(search_tool_name, &token) {
        Ok(()) => Ok(answer(true, None)),
        Err(denial) => Ok(answer(false, Some(denial.as_ref()))),
    }
}

fn search_team(search_tool_name: &str, fields: Option<&TeamFields>) -> HelperResult {
    let team: Option<LiteLLMTeamTable> = match fields {
        Some(team_fields) => {
            let mut object = BTreeMap::new();
            object.insert("team_id", json!(team_fields.team_id.as_str()));
            if let Some(id) = &team_fields.object_permission_id {
                object.insert("object_permission_id", json!(id));
            }
            if let Some(permission) = &team_fields.permission {
                object.insert("object_permission", permission_data(permission));
            }
            Some(serde_json::from_value(json!(object))?)
        }
        None => None,
    };
    match can_team_call_search_tool(search_tool_name, team.as_ref()) {
        Ok(()) => Ok(answer(true, None)),
        Err(denial) => Ok(answer(false, Some(denial.as_ref()))),
    }
}

// the repository fixture: cached rows first, then one batched read for the misses
fn managed_rows(rows: &[RowFixture], rounds: &[Vec<String>]) -> HelperResult {
    let fixture = Fixture {
        cached: Mutex::new(BTreeMap::new()),
        rows: rows
            .iter()
            .map(|row| (row.id.clone(), vector_row(row)))
            .collect(),
        queries: Mutex::new(Vec::new()),
    };
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let mut answers = Vec::with_capacity(rounds.len());
    for uuids in rounds {
        let mut queries = fixture
            .queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        queries.clear();
        drop(queries);
        let found = runtime
            .block_on(vector_store_rows_by_uuids(uuids, true, &fixture, &fixture))
            .or_value_error()?;
        answers.push(json!({
            "vector_store_ids": found
                .iter()
                .map(|row| row.vector_store_id.as_str())
                .collect::<Vec<_>>(),
            "queries": fixture
                .queries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }));
    }
    Ok(json!(answers))
}

fn answer(allowed: bool, denial: Option<&ProxyException>) -> Value {
    let error = denial.map_or(Value::Null, |exception| {
        json!({
            "status": exception.code.parse::<u16>().unwrap_or_default(),
            "error_type": exception.error_type.as_str(),
            "param": &exception.param,
            "code": exception.code.as_str(),
        })
    });
    json!({"allowed": allowed, "error": error})
}

fn vector_row(fixture: &RowFixture) -> VectorStoreRow {
    VectorStoreRow {
        model_dump: Some(JsonData::Object(fixture.model_dump.clone())),
        dict: None,
        fields: BTreeMap::new(),
    }
}

fn permission_data(fields: &PermissionFields) -> Value {
    let mut object = BTreeMap::new();
    object.insert(
        "object_permission_id",
        json!(fields.object_permission_id.as_str()),
    );
    let search_tools = fields
        .search_tools
        .as_ref()
        .map_or(Value::Null, |tools| json!(tools));
    object.insert("search_tools", search_tools);
    json!(object)
}

// the cache and the repository of one fixture: hits are whatever an earlier round cached
struct Fixture {
    cached: Mutex<BTreeMap<String, ManagedVectorStoresTable>>,
    rows: Vec<(String, VectorStoreRow)>,
    queries: Mutex<Vec<Value>>,
}

#[async_trait]
impl ManagedVectorStoreCache for Fixture {
    async fn read(&self, key: &str) -> Result<Option<ManagedVectorStoresTable>, PayloadError> {
        let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(cached.get(key).cloned())
    }

    fn management_ttl(&self) -> f64 {
        MANAGEMENT_TTL_SECONDS
    }

    async fn write(
        &self,
        key: &str,
        row: &ManagedVectorStoresTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), row.clone());
        Ok(())
    }
}

#[async_trait]
impl ManagedVectorStoreRows for Fixture {
    async fn by_uuids(
        &self,
        query: ManagedRowsQuery,
    ) -> Result<Vec<VectorStoreRow>, VectorStoreRowsFailure> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"uuids": &query.uuids, "take": query.take}));
        Ok(self
            .rows
            .iter()
            .filter(|(id, _)| query.uuids.contains(id))
            .map(|(_, row)| row.clone())
            .collect())
    }
}
