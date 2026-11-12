use serde::Deserialize;
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::OrHelperError as _;

use adapter::Adapter;
use warnings::Warnings;

mod adapter;
mod budget;
mod effort;
mod effort_param;
mod fixture;
mod reasoning;
mod speed;
mod supported_params;
mod tests;
mod warnings;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    CapThinkingBudgetToMaxTokens(budget::CapStep),
    GetSupportedOpenaiParams(supported_params::SupportedParamsStep),
    MapReasoningEffort(reasoning::MapStep),
    MaybeDropSpeedParam(speed::SpeedDropStep),
    ModelSupportsEffortParam(effort_param::EffortParamStep),
    ModelSupportsSpeedParam(speed::SpeedSupportStep),
    RaiseInvalidReasoningEffort(effort::InvalidStep),
    ValidateEffortForModel(effort::ValidateStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: fixture::FixtureInput,
    steps: Vec<Step>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let fixture = fixture::Fixture::load(&input.fixture).or_config_error()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let answers = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(run_steps(input, &fixture, &warnings))
        })?;
        Ok(answers)
    }
}

async fn run_steps(
    input: &Input,
    fixture: &fixture::Fixture,
    warnings: &Warnings,
) -> crate::helpers::HelperResult {
    let resolution = fixture.resolution();
    let mut keys = betterlite_models::model_catalog_keys::ModelCostKeys::default();
    let dynamic = betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
    let adapter = Adapter {
        catalogs: fixture.catalogs(),
        budgets: fixture.budgets,
        warnings,
    };
    let mut answers: Vec<Value> = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
        answers.push(adapter.answer(step, &mut environment).await?);
    }
    Ok(Value::Array(answers))
}
