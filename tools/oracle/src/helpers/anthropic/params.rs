use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::params::output_config::{
    OutputConfigRequest, apply as apply_output_config,
};
use betterlite_anthropic::params::{DEFAULT_MAX_TOKENS, ParamsRequest, map_openai_params};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;
use tracing::subscriber;
use warnings::Warnings;

mod fixture;
mod tests;
mod warnings;

const PROVIDER: &str = "anthropic";

// The fixture names no huggingface model, so every step resolves through these two
type Environment<'env> = CapabilityEnvironment<'env, NoDynamicMetadata, fixture::NoHfTransport>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: fixture::FixtureInput,
    #[serde(default)]
    default_max_tokens: Option<i64>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    MapOpenaiParams {
        model: String,
        #[serde(default)]
        non_default_params: IndexMap<String, Json>,
        #[serde(default)]
        optional_params: IndexMap<String, Json>,
        drop_params: bool,
        global_drop_params: bool,
    },
    ApplyOutputConfig {
        model: String,
        #[serde(default)]
        optional_params: IndexMap<String, Json>,
        global_drop_params: bool,
    },
}

struct StepContext<'context> {
    budgets: &'context Settings,
    catalogs: &'context CatalogSources<'context>,
    default_max_tokens: i64,
    warnings: &'context Warnings,
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
        subscriber::with_default(subscriber, || {
            runtime.block_on(run_steps(input, &fixture, &warnings))
        })
    }
}

async fn run_steps(input: &Input, fixture: &fixture::Fixture, warnings: &Warnings) -> HelperResult {
    let resolution = fixture.resolution();
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let catalogs = fixture.catalogs();
    let budgets = Settings::default();
    let context = StepContext {
        budgets: &budgets,
        catalogs: &catalogs,
        default_max_tokens: input.default_max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        warnings,
    };
    let mut answers = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        warnings.clear();
        let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
        // LiteLLM raises here; the case compares that the step raised, not the message
        answers.push(
            answer(step, &context, &mut environment)
                .await
                .unwrap_or_else(|_| json!({"error": {"raised": true}})),
        );
    }
    Ok(Value::Array(answers))
}

async fn answer(
    step: &Step,
    context: &StepContext<'_>,
    environment: &mut Environment<'_>,
) -> Result<Value, Box<HelperError>> {
    match step {
        Step::MapOpenaiParams {
            model,
            non_default_params: non_default,
            optional_params: optional,
            drop_params,
            global_drop_params,
        } => {
            let request = ParamsRequest {
                model,
                provider: PROVIDER,
                drop: DropParams {
                    global: *global_drop_params,
                    call: *drop_params,
                },
                budgets: context.budgets,
                default_max_tokens: context.default_max_tokens,
                catalogs: context.catalogs,
            };
            answer_map(&request, non_default, optional, context, environment).await
        }
        Step::ApplyOutputConfig {
            model,
            optional_params: optional,
            global_drop_params,
        } => answer_output(model, *global_drop_params, optional, context, environment).await,
    }
}

#[derive(Serialize)]
struct MapAnswer {
    optional_params: IndexMap<String, Json>,
    warning: Option<String>,
}

async fn answer_map(
    request: &ParamsRequest<'_>,
    non_default: &IndexMap<String, Json>,
    optional: &IndexMap<String, Json>,
    context: &StepContext<'_>,
    environment: &mut Environment<'_>,
) -> Result<Value, Box<HelperError>> {
    let mut mapped = optional.clone();
    map_openai_params(request, non_default, &mut mapped, environment)
        .await
        .or_value_error()?;
    let answer = MapAnswer {
        optional_params: mapped,
        warning: context.warnings.first(),
    };
    serde_json::to_value(answer).or_value_error()
}

#[derive(Serialize)]
struct OutputSide {
    output_config: Option<Json>,
    output_config_present: bool,
}

#[derive(Serialize)]
struct OutputAnswer {
    data: OutputSide,
    optional_params: OutputSide,
    warning: Option<String>,
}

fn output_side(params: &IndexMap<String, Json>) -> OutputSide {
    OutputSide {
        output_config: params.get("output_config").cloned(),
        output_config_present: params.contains_key("output_config"),
    }
}

async fn answer_output(
    model: &str,
    global_drop_params: bool,
    optional_params: &IndexMap<String, Json>,
    context: &StepContext<'_>,
    environment: &mut Environment<'_>,
) -> Result<Value, Box<HelperError>> {
    let request = OutputConfigRequest {
        model,
        provider: PROVIDER,
        global_drop: global_drop_params,
    };
    // transform_request builds its body from {**optional_params} before this runs
    let mut updated_data = optional_params.clone();
    let mut updated_optional = optional_params.clone();
    apply_output_config(
        &request,
        &mut updated_data,
        &mut updated_optional,
        environment,
        context.catalogs,
    )
    .await
    .or_value_error()?;
    let answer = OutputAnswer {
        data: output_side(&updated_data),
        optional_params: output_side(&updated_optional),
        warning: context.warnings.first(),
    };
    serde_json::to_value(answer).or_value_error()
}
