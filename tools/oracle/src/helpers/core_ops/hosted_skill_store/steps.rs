use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::skills::store::repository::SkillRepository;
use betterlite_core::skills::store::{
    DEFAULT_PAGE_LIMIT, DEFAULT_PAGE_OFFSET, HostedSkillStore, SkillPage, StoreError,
};
use betterlite_db::managed_resource_rows::skills::SkillTable;
use serde_json::{Value, json};

use super::principal::{new_skill, principal_of};
use super::{Step, invalid_input};

fn page_of(step: &Step) -> SkillPage {
    SkillPage {
        limit: step.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        offset: step.offset.unwrap_or(DEFAULT_PAGE_OFFSET),
    }
}

fn token(error: &StoreError) -> &'static str {
    match error {
        StoreError::NotFound { .. } => "not_found",
        StoreError::NoDatabase => "no_database",
        StoreError::MissingIdentity => "missing_identity",
        StoreError::Repository { .. } | StoreError::Metadata(_) | StoreError::Cache(_) => {
            "exception"
        }
    }
}

fn row_json(row: &SkillTable) -> Result<Value, Box<crate::helpers::HelperError>> {
    match serde_json::to_value(row) {
        Ok(value) => Ok(value),
        Err(error) => Err(invalid_input(error.to_string())),
    }
}

fn rows_json(rows: &[SkillTable]) -> Result<Value, Box<crate::helpers::HelperError>> {
    rows.iter()
        .map(row_json)
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn invalid(step: &Step) -> Value {
    json!({"action": step.action.as_str(), "error": "invalid_input"})
}

fn error_answer(action: &str, error: &StoreError) -> Value {
    json!({"action": action, "error": token(error)})
}

fn row_answer(
    action: &str,
    outcome: Result<SkillTable, StoreError>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    match outcome {
        Ok(row) => row_json(&row).map(|row| json!({"action": action, "row": row})),
        Err(error) => Ok(error_answer(action, &error)),
    }
}

pub(super) async fn run_steps<R: SkillRepository>(
    store: &HostedSkillStore<R>,
    steps: &[Step],
) -> Result<Vec<Value>, Box<crate::helpers::HelperError>> {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        results.push(run_step(store, step).await?);
    }
    Ok(results)
}

async fn run_step<R: SkillRepository>(
    store: &HostedSkillStore<R>,
    step: &Step,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let caller = step.principal.as_ref().map(principal_of).transpose()?;
    let action = step.action.as_str();
    match action {
        "list" | "search" => return list_step(store, step, caller.as_ref()).await,
        "get" | "delete" | "fetch" => return skill_step(store, step, caller.as_ref()).await,
        "create" => {}
        _ => return Ok(json!({"action": action, "error": "invalid_input"})),
    }
    let Some(fields) = step.data.as_ref() else {
        return Ok(invalid(step));
    };
    let data = new_skill(fields)?;
    row_answer(
        action,
        store
            .create_skill(&data, step.user_id.as_deref(), caller.as_ref())
            .await,
    )
}

async fn list_step<R: SkillRepository>(
    store: &HostedSkillStore<R>,
    step: &Step,
    caller: Option<&Principal>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let action = step.action.as_str();
    let outcome = if action == "search" {
        store.list_skills_for_search(caller).await
    } else {
        store.list_skills(page_of(step), caller).await
    };
    match outcome {
        Ok(rows) => rows_json(&rows).map(|rows| json!({"action": action, "rows": rows})),
        Err(error) => Ok(error_answer(action, &error)),
    }
}

async fn skill_step<R: SkillRepository>(
    store: &HostedSkillStore<R>,
    step: &Step,
    caller: Option<&Principal>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let Some(skill_id) = step.skill_id.as_deref() else {
        return Ok(invalid(step));
    };
    let action = step.action.as_str();
    if action == "get" {
        return row_answer(action, store.get_skill(skill_id, caller).await);
    }
    if action == "delete" {
        return match store.delete_skill(skill_id, caller).await {
            Ok(deleted) => Ok(json!({
                "action": action,
                "result": {"id": deleted.id, "type": deleted.r#type},
            })),
            Err(error) => Ok(error_answer(action, &error)),
        };
    }
    let Some(row) = store.fetch_skill_from_db(skill_id, caller).await else {
        return Ok(json!({"action": action, "row": null}));
    };
    row_json(&row).map(|row| json!({"action": action, "row": row}))
}
