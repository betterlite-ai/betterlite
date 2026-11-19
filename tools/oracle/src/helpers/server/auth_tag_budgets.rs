use std::collections::BTreeMap;

use betterlite_auth::auth_tag_budgets::groups::{ModelAccessGroupBudget, group_budgets_batch};
use betterlite_auth::auth_tag_budgets::registry::{RegistryKind, load_registry};
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow, RowName};
use betterlite_auth::auth_tag_budgets::tags::get_tag_objects_batch;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::fixture::Fixture;

mod fixture;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Tags,
    Groups,
    TagRegistry,
    GroupRegistry,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default = "default_database")]
    database: bool,
    #[serde(default)]
    names: Vec<String>,
    // rows an earlier request already cached
    #[serde(default)]
    cached: Vec<Value>,
    #[serde(default)]
    rows: Vec<Value>,
    #[serde(default)]
    generated: Option<Generated>,
    #[serde(default)]
    fail_query: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Generated {
    prefix: String,
    count: usize,
}

const fn default_database() -> bool {
    true
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::new(input).or_value_error()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(answer(input, &fixture))
    }
}

async fn answer(input: &Input, fixture: &Fixture) -> HelperResult {
    let mut tags = BTreeMap::new();
    let mut groups = BTreeMap::new();
    let mut registry = Value::Null;
    match &input.function {
        Function::Tags => {
            let found = get_tag_objects_batch(&input.names, input.database, fixture, fixture)
                .await
                .or_value_error()?;
            for (name, tag) in found {
                tags.insert(name, tag_view(&tag)?);
            }
        }
        Function::Groups => {
            let found = group_budgets_batch(&input.names, input.database, fixture, fixture)
                .await
                .or_value_error()?;
            for (name, budget) in found {
                groups.insert(name, group_view(&budget)?);
            }
        }
        Function::TagRegistry => {
            registry = registry_answer::<TagTable>(RegistryKind::Tag, fixture).await?;
        }
        Function::GroupRegistry => {
            registry =
                registry_answer::<ModelAccessGroupBudgetRow>(RegistryKind::Group, fixture).await?;
        }
    }
    let effects = fixture.snapshot();
    Ok(json!({
        "tags": tags,
        "groups": groups,
        "registry": registry,
        "reads": effects.reads,
        "writes": effects.writes,
        "queries": effects.queries,
        "takes": effects.takes,
    }))
}

// LiteLLM's _load_tag_registry and _load_model_access_group_registry: the names one row read can prove exist.
async fn registry_answer<Row>(kind: RegistryKind, fixture: &Fixture) -> HelperResult
where
    Row: RowName,
    Fixture: BudgetRows<Row>,
{
    let table: &dyn BudgetRows<Row> = fixture;
    let loaded = load_registry(kind, fixture, table).await.or_value_error()?;
    Ok(loaded.map_or(Value::Null, |names| json!(names)))
}

// the fields both sides compare: the projection keeps the ceiling as the float LiteLLM returns
fn tag_view(tag: &TagTable) -> Result<Value, serde_json::Error> {
    let row = serde_json::to_value(tag)?;
    Ok(json!({
        "models": row.get("models").cloned(),
        "spend": row.get("spend").and_then(Value::as_f64),
        "max_budget": row
            .get("litellm_budget_table")
            .and_then(|budget| budget.get("max_budget"))
            .and_then(Value::as_f64),
    }))
}

fn group_view(budget: &ModelAccessGroupBudget) -> Result<Value, serde_json::Error> {
    let row = serde_json::to_value(budget)?;
    Ok(json!({
        "spend": row.get("spend").and_then(Value::as_f64),
        "max_budget": row.get("max_budget").and_then(Value::as_f64),
    }))
}
