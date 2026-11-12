use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::sampling::SamplingParam;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::OrHelperError as _;

use adapter::Adapter;
use warnings::Warnings;

mod adapter;
mod adaptive;
mod beta;
mod budget;
mod disabled;
mod fixture;
mod provider;
mod tests;
mod warnings;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    ProviderQualifiedCatalog {
        model: String,
        custom_llm_provider: String,
    },
    AdaptiveThinking {
        model: String,
        custom_llm_provider: String,
        params: OptionalParams,
    },
    ForcedToolDowngrade {
        model: String,
        tool_choice: Json,
        drop_params: bool,
        global_drop_params: bool,
    },
    SamplingCapability {
        model: String,
        param: SamplingParam,
        value: Json,
        output_key: String,
        drop_params: bool,
        global_drop_params: bool,
    },
    LegacyBudgetEffort {
        model: String,
        custom_llm_provider: String,
        budget_tokens: i64,
    },
    DisabledThinkingWarning {
        model: String,
        custom_llm_provider: String,
        params: OptionalParams,
    },
    EffortBetaGate {
        model: String,
        custom_llm_provider: String,
        params: OptionalParams,
    },
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
