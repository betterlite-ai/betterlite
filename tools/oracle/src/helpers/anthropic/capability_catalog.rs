use serde::Deserialize;
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt as _;

use betterlite_oracle_helper_api::error::OrHelperError as _;

use adapter::Adapter;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;
use tracing::subscriber;
use warnings::Warnings;

mod adapter;
mod budget;
mod drop_params;
mod fixture;
mod provider;
mod tests;
mod thinking;
mod warnings;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    ProviderQualifiedCatalog(provider::CatalogStep),
    AdaptiveThinking(thinking::ThinkingStep),
    ForcedToolDowngrade(drop_params::ForcedToolStep),
    SamplingCapability(drop_params::SamplingStep),
    LegacyBudgetEffort(budget::BudgetStep),
    DisabledThinkingWarning(thinking::ThinkingStep),
    EffortBetaGate(thinking::ThinkingStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: fixture::FixtureInput,
    steps: Vec<Step>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = fixture::Fixture::load(&input.fixture).or_config_error()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let answers = subscriber::with_default(subscriber, || {
            runtime.block_on(run_steps(input, &fixture, &warnings))
        })?;
        Ok(answers)
    }
}

async fn run_steps(input: &Input, fixture: &fixture::Fixture, warnings: &Warnings) -> HelperResult {
    let resolution = fixture.resolution();
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let adapter = Adapter {
        catalogs: fixture.catalogs(),
        thresholds: fixture.thresholds,
        warnings,
    };
    let mut answers: Vec<Value> = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
        answers.push(adapter.answer(step, &mut environment).await?);
    }
    Ok(Value::Array(answers))
}
