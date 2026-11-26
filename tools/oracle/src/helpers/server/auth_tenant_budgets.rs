mod fixtures;
mod tests;

use betterlite_auth::auth_tag_budgets::cache::TagBudgetCache;
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use betterlite_auth::auth_tenant_budgets::access_groups;
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

// the scripted collaborators one case hands to the production functions
struct Answers<'run> {
    reader: &'run Seeded,
    recorder: &'run Recorder,
    admission: TenantAdmission<'run>,
}

async fn answer(call: &Call) -> Result<Value, Failure> {
    let reader = Seeded::new(&call.spends);
    let recorder = Recorder::default();
    let answers = Answers {
        reader: &reader,
        recorder: &recorder,
        admission: TenantAdmission {
            reader: &reader,
            alerts: Some(&recorder),
        },
    };
    let token = token_of(call)?;
    let world = TagWorld::new(call.tag_rows.clone(), call.group_rows.clone());
    match call.function {
        Function::ProjectHard => project_hard(call, &answers, token.as_ref()).await,
        Function::ProjectSoft => Ok(project_soft(call, &answers, token.as_ref())),
        Function::Organization => organization_value(call, &answers, token).await,
        Function::Tags => tags_value(call, &answers, &world).await,
        Function::Groups => groups_value(call, &answers, &world).await,
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
    answers: &Answers<'_>,
    token: Option<&UserAPIKeyAuth>,
) -> Result<Value, Failure> {
    let Some(row) = call.project.as_ref() else {
        return recorded(answers, Ok(()), None);
    };
    let outcome = project::max_budget_check(row, token, &answers.admission).await;
    recorded(answers, outcome, None)
}

fn project_soft(call: &Call, answers: &Answers<'_>, token: Option<&UserAPIKeyAuth>) -> Value {
    let Some(row) = call.project.as_ref() else {
        return json!({"logs": [], "alerts": Value::Array(answers.recorder.alerts())});
    };
    let outcome = project::soft_budget_check(row, token, Some(answers.recorder));
    json!({"logs": soft_logs(outcome, row), "alerts": Value::Array(answers.recorder.alerts())})
}

async fn organization_value(
    call: &Call,
    answers: &Answers<'_>,
    mut token: Option<UserAPIKeyAuth>,
) -> Result<Value, Failure> {
    let world = org_world(call.organization.clone(), call.org_raises);
    let lookup = call.database.then(|| org_lookup(&world));
    let outcome = organization::max_budget_check(
        token.as_mut(),
        call.team.as_ref(),
        lookup.as_ref(),
        &answers.admission,
    )
    .await;
    let carried = match token.as_ref() {
        Some(token) => carried_record(token)?,
        None => Value::Null,
    };
    recorded(answers, outcome, Some(carried))
}

async fn tags_value(
    call: &Call,
    answers: &Answers<'_>,
    world: &TagWorld,
) -> Result<Value, Failure> {
    let body = call.request_body.clone().unwrap_or_default();
    let names = tags_from_request_body(&body);
    let sources = sources(world, call.database);
    let outcome = tags::max_budget_check(&names, &sources, &answers.admission).await;
    recorded(answers, outcome, None)
}

async fn groups_value(
    call: &Call,
    answers: &Answers<'_>,
    world: &TagWorld,
) -> Result<Value, Failure> {
    let sources = sources(world, call.database);
    let outcome = access_groups::access_group_max_budget_check(
        &call.matched_model_access_groups,
        &sources,
        &answers.admission,
    )
    .await;
    recorded(answers, outcome, None)
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

// the reads and alerts the case compares, the denial the check settled on and, for the organization, the carried key
fn recorded(
    answers: &Answers<'_>,
    outcome: Result<(), TenantBudgetError>,
    carried: Option<Value>,
) -> Result<Value, Failure> {
    let denial = match outcome {
        Ok(()) => Value::Null,
        Err(TenantBudgetError::Denied(denied)) => denial_json(&denied),
        Err(error @ (TenantBudgetError::Spend(_) | TenantBudgetError::Tag(_))) => {
            return Err(Failure::Admission(error));
        }
    };
    let mut record = json!({
        "reads": Value::Array(answers.reader.reads()),
        "alerts": Value::Array(answers.recorder.alerts()),
        "denial": denial,
    });
    if let Some(carried) = carried {
        record
            .as_object_mut()
            .expect("the recorded answer is an object")
            .insert("carried".to_owned(), carried);
    }
    Ok(record)
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
