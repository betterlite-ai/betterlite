mod fixtures;
mod tests;

use betterlite_auth::auth_tag_budgets::cache::TagBudgetCache;
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use betterlite_auth::auth_tenant_budgets::error::TenantBudgetError;
use betterlite_auth::auth_tenant_budgets::project;
use betterlite_auth::auth_tenant_budgets::tags;
use betterlite_auth::auth_tenant_budgets::{TenantAdmission, TenantSources, organization};
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_destination::parameters::tags_from_request_body;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fixtures::{
    Call, CarriedError, Function, Input, Recorder, Seeded, TagWorld, carried_record, denial_json,
    org_lookup, org_world, soft_logs,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answers = Vec::new();
        for call in &input.calls {
            answers.push(match runtime.block_on(answer(call)) {
                Ok(value) => json!({"ok": true, "value": value}),
                Err(_failure) => json!({"ok": false}),
            });
        }
        Ok(Value::Array(answers))
    }
}

async fn answer(call: &Call) -> Result<Value, Failure> {
    let reader = Seeded::new(&call.spends);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let token = token_of(call)?;
    let world = TagWorld::new(call.tag_rows.clone(), call.group_rows.clone());
    match call.function {
        Function::ProjectHard => {
            project_hard(call, &reader, &recorder, token.as_ref(), &admission).await
        }
        Function::ProjectSoft => Ok(project_soft(call, &recorder, token.as_ref())),
        Function::Organization => {
            organization_value(call, &reader, &recorder, token, &admission).await
        }
        Function::Tags => tags_value(call, &reader, &recorder, &world, &admission).await,
        Function::Groups => groups_value(call, &reader, &recorder, &world, &admission).await,
    }
}

fn token_of(call: &Call) -> Result<Option<UserAPIKeyAuth>, Failure> {
    let Some(fields) = call.token.clone() else {
        return Ok(None);
    };
    Ok(Some(UserAPIKeyAuth::construct(AuthInput::Dict(fields))?))
}

async fn project_hard(
    call: &Call,
    reader: &Seeded,
    recorder: &Recorder,
    token: Option<&UserAPIKeyAuth>,
    admission: &TenantAdmission<'_>,
) -> Result<Value, Failure> {
    let outcome = match call.project.as_ref() {
        Some(row) => project::max_budget_check(row, token, admission).await,
        None => Ok(()),
    };
    Ok(recorded(reader, recorder, outcome)?.whole())
}

fn project_soft(call: &Call, recorder: &Recorder, token: Option<&UserAPIKeyAuth>) -> Value {
    let Some(row) = call.project.as_ref() else {
        return json!({"logs": [], "alerts": recorder.alerts()});
    };
    let outcome = project::soft_budget_check(row, token, Some(recorder));
    json!({"logs": soft_logs(outcome, row), "alerts": recorder.alerts()})
}

async fn organization_value(
    call: &Call,
    reader: &Seeded,
    recorder: &Recorder,
    mut token: Option<UserAPIKeyAuth>,
    admission: &TenantAdmission<'_>,
) -> Result<Value, Failure> {
    let world = org_world(call.organization.clone(), call.org_raises);
    let lookup = call.database.then(|| org_lookup(&world));
    let outcome = organization::max_budget_check(
        token.as_mut(),
        call.team.as_ref(),
        lookup.as_ref(),
        admission,
    )
    .await;
    let carried = match token.as_ref() {
        Some(token) => carried_record(token)?,
        None => Value::Null,
    };
    let recorded = recorded(reader, recorder, outcome)?;
    Ok(json!({
        "reads": recorded.reads,
        "alerts": recorded.alerts,
        "denial": recorded.denial,
        "carried": carried,
    }))
}

async fn tags_value(
    call: &Call,
    reader: &Seeded,
    recorder: &Recorder,
    world: &TagWorld,
    admission: &TenantAdmission<'_>,
) -> Result<Value, Failure> {
    let empty = RequestData::new();
    let body = call.request_body.as_ref().unwrap_or(&empty);
    let tag_names = tags_from_request_body(body);
    let sources = sources(world, call.database);
    let outcome = tags::max_budget_check(&tag_names, &sources, admission).await;
    Ok(recorded(reader, recorder, outcome)?.whole())
}

async fn groups_value(
    call: &Call,
    reader: &Seeded,
    recorder: &Recorder,
    world: &TagWorld,
    admission: &TenantAdmission<'_>,
) -> Result<Value, Failure> {
    let sources = sources(world, call.database);
    let outcome = tags::model_access_group_max_budget_check(
        &call.matched_model_access_groups,
        &sources,
        admission,
    )
    .await;
    Ok(recorded(reader, recorder, outcome)?.whole())
}

fn sources(world: &TagWorld, database: bool) -> TenantSources<'_> {
    let cache: &dyn TagBudgetCache = world;
    let tag_rows: &dyn BudgetRows<TagTable> = world;
    let group_rows: &dyn BudgetRows<ModelAccessGroupBudgetRow> = world;
    TenantSources {
        database,
        organizations: None,
        cache: Some(cache),
        tags: Some(tag_rows),
        groups: Some(group_rows),
    }
}

// the reads and alerts the case compares, with the denial either budget check settled on
struct Recorded {
    reads: Value,
    alerts: Value,
    denial: Value,
}

impl Recorded {
    fn whole(self) -> Value {
        json!({"reads": self.reads, "alerts": self.alerts, "denial": self.denial})
    }
}

fn recorded(
    reader: &Seeded,
    recorder: &Recorder,
    outcome: Result<(), TenantBudgetError>,
) -> Result<Recorded, Failure> {
    match outcome {
        Ok(()) => Ok(Recorded {
            reads: Value::Array(reader.reads()),
            alerts: Value::Array(recorder.alerts()),
            denial: Value::Null,
        }),
        Err(TenantBudgetError::Denied(denial)) => Ok(Recorded {
            reads: Value::Array(reader.reads()),
            alerts: Value::Array(recorder.alerts()),
            denial: denial_json(&denial),
        }),
        Err(error @ (TenantBudgetError::Spend(_) | TenantBudgetError::Tag(_))) => {
            Err(Failure::Admission(error))
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Admission(#[from] TenantBudgetError),
    #[error(transparent)]
    Carried(#[from] CarriedError),
}
