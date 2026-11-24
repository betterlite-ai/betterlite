use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_server::key_list_filters::alias_scope::apply_non_admin_alias_scope;
use betterlite_server::key_list_filters::error::KeyListFiltersError;
use betterlite_server::key_list_filters::selectors::KeyListSelectors;
use betterlite_server::key_list_filters::teams::{
    admin_team_ids, fetch_user_team_objects, list_permission_team_ids, member_team_ids,
};
use betterlite_server::key_list_filters::validate::validate_key_list_check;
use betterlite_server::team_access::caller::TeamCaller;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::fakes::{ScriptStore, ScriptTeams, team_lookup};
use super::{Call, CallFailed, Caller, Selectors};

pub(super) fn answer(call: &Call) -> Result<Value, CallFailed> {
    match call {
        Call::Validate {
            caller,
            selectors,
            user_row,
            key_row,
            key_fails,
        } => validate_answer(
            caller,
            selectors,
            user_row.as_ref(),
            key_row.as_ref(),
            *key_fails,
        ),
        Call::Teams {
            user,
            teams,
            teams_fail,
        } => teams_answer(user.as_ref(), teams.as_deref(), *teams_fail),
        Call::Projections { caller, teams } => projections_answer(caller, teams),
        Call::AliasScope {
            caller,
            user_row,
            query_params,
            where_parts,
        } => alias_scope_answer(
            caller,
            user_row.as_ref(),
            query_params.clone(),
            where_parts.clone(),
        ),
    }
}

fn validate_answer(
    fields: &Caller,
    selectors: &Selectors,
    user_row: Option<&Value>,
    key_row: Option<&Value>,
    key_fails: bool,
) -> Result<Value, CallFailed> {
    let auth = auth_of(fields)?;
    let store = ScriptStore::new(user_of(user_row)?, key_of(key_row)?, key_fails);
    let teams = team_lookup();
    let wanted = wanted_of(selectors);
    let outcome = runtime()?.block_on(validate_key_list_check(&auth, &wanted, &store, &teams));
    let (value, error): (Value, Option<Value>) = match outcome {
        Ok(None) => (Value::Null, None),
        Ok(Some(row)) => (row_summary(&row), None),
        Err(failure) => (Value::Null, Some(error_json(&failure))),
    };
    Ok(json!({
        "value": value,
        "error": error,
        "warnings": [],
        "effects": store.trace(),
    }))
}

fn teams_answer(
    user: Option<&Value>,
    rows: Option<&[Value]>,
    fails: bool,
) -> Result<Value, CallFailed> {
    let found = user_of(user)?;
    let fake = ScriptTeams {
        rows: rows.unwrap_or_default().to_vec(),
        fails,
        trace: Mutex::new(Vec::new()),
    };
    let outcome = runtime()?.block_on(fetch_user_team_objects(found.as_ref(), &fake));
    let (value, error): (Value, Option<Value>) = match outcome {
        Ok(teams) => (team_ids_of(&teams), None),
        Err(failure) => (Value::Null, Some(error_json(&failure))),
    };
    Ok(json!({
        "value": value,
        "error": error,
        "warnings": [],
        "effects": fake.trace(),
    }))
}

fn projections_answer(fields: &Caller, rows: &[Value]) -> Result<Value, CallFailed> {
    let auth = auth_of(fields)?;
    let caller = TeamCaller::try_from(&auth)?;
    let teams = teams_of(rows)?;
    let projection = json!({
        "admins": admin_team_ids(&caller, &teams),
        "list_permission": list_permission_team_ids(&caller, &teams),
        "members": member_team_ids(&caller, &teams),
    });
    Ok(json!({
        "value": projection,
        "error": null,
        "warnings": [],
        "effects": [],
    }))
}

fn alias_scope_answer(
    fields: &Caller,
    user_row: Option<&Value>,
    query_params: Vec<JsonData>,
    where_parts: Vec<String>,
) -> Result<Value, CallFailed> {
    let auth = auth_of(fields)?;
    let caller = TeamCaller::try_from(&auth)?;
    let store = ScriptStore::new(user_of(user_row)?, None, false);
    let mut params = query_params;
    let mut filters = where_parts;
    let outcome = runtime()?.block_on(apply_non_admin_alias_scope(
        &caller,
        &store,
        &mut params,
        &mut filters,
    ));
    let (value, error): (Value, Option<Value>) = match outcome {
        Ok(()) => (json!({"params": params, "where_parts": filters}), None),
        Err(failure) => (Value::Null, Some(error_json(&failure))),
    };
    Ok(json!({
        "value": value,
        "error": error,
        "warnings": [],
        "effects": store.trace(),
    }))
}

fn auth_of(fields: &Caller) -> Result<UserAPIKeyAuth, CallFailed> {
    let mut values: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(user_id) = &fields.user_id {
        values.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
    }
    if let Some(role) = fields.user_role {
        values.insert("user_role".to_owned(), JsonData::String(role.to_string()));
    }
    Ok(UserAPIKeyAuth::construct(AuthInput::Dict(values))?)
}

fn wanted_of(selectors: &Selectors) -> KeyListSelectors<'_> {
    KeyListSelectors {
        user_id: selectors.user_id.as_deref(),
        team_id: selectors.team_id.as_deref(),
        organization_id: selectors.organization_id.as_deref(),
        key_alias: selectors.key_alias.as_deref(),
        key_hash: selectors.key_hash.as_deref(),
    }
}

fn user_of(row: Option<&Value>) -> Result<Option<LiteLLMUserTable>, CallFailed> {
    match row {
        Some(value) => Ok(Some(serde_json::from_value(value.clone())?)),
        None => Ok(None),
    }
}

fn key_of(row: Option<&Value>) -> Result<Option<LiteLLMVerificationToken>, CallFailed> {
    match row {
        Some(value) => Ok(Some(serde_json::from_value(value.clone())?)),
        None => Ok(None),
    }
}

fn teams_of(rows: &[Value]) -> Result<Vec<LiteLLMTeamTable>, CallFailed> {
    let decoded: Result<Vec<Option<LiteLLMTeamTable>>, TeamRecordError> = rows
        .iter()
        .map(|row| {
            let record = TeamRecord::parse(row.to_string().as_bytes())?;
            decode_team_record(record.as_ref())
        })
        .collect();
    Ok(decoded
        .map_err(CallFailed::from)?
        .into_iter()
        .flatten()
        .collect())
}

fn row_summary(row: &LiteLLMUserTable) -> Value {
    let organizations = row
        .organization_memberships
        .as_ref()
        .map_or(Value::Null, |memberships| {
            Value::Array(
                memberships
                    .iter()
                    .map(|membership| Value::String(membership.organization_id.clone()))
                    .collect(),
            )
        });
    json!({
        "user_id": row.user_id.clone(),
        "teams": row.teams.clone(),
        "organization_ids": organizations,
    })
}

fn team_ids_of(teams: &[LiteLLMTeamTable]) -> Value {
    Value::Array(
        teams
            .iter()
            .map(|team| Value::String(team.fields.base.team_id.clone()))
            .collect(),
    )
}

fn error_json(failure: &KeyListFiltersError) -> Value {
    let status = failure.status();
    match failure.proxy_exception() {
        Some(raised) => json!({
            "status": status,
            "type": raised.error_type,
            "param": raised.param,
            "code": raised.code,
        }),
        None => json!({
            "status": status,
            "type": Value::Null,
            "param": Value::Null,
            "code": Value::Null,
        }),
    }
}

fn runtime() -> Result<Runtime, CallFailed> {
    Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(CallFailed::from)
}
