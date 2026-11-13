use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::output_config::{
    OutputConfigRequest, apply as apply_output_config,
};
use betterlite_anthropic::params::{DEFAULT_MAX_TOKENS, ParamsRequest, map_openai_params};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{HelperError, OrHelperError as _};

use warnings::Warnings;

mod fixture;
mod tests;
mod warnings;

const PROVIDER: &str = "anthropic";

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
        data: IndexMap<String, Json>,
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
        tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(run_steps(input, &fixture, &warnings))
        })
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
        answers.push(answer(step, &context, &mut environment).await?);
    }
    Ok(Value::Array(answers))
}

// LiteLLM prefixes the exception's own name onto the message it carries
fn helper_error(error: &ParamsError) -> Box<HelperError> {
    let class = match error {
        ParamsError::Speed(_) | ParamsError::Sampling(_) | ParamsError::ForcedTool(_) => {
            "UnsupportedParamsError"
        }
        ParamsError::Base(_)
        | ParamsError::Context(_)
        | ParamsError::Effort(_)
        | ParamsError::Options(_)
        | ParamsError::Reasoning(_)
        | ParamsError::Schema(_)
        | ParamsError::Thinking(_)
        | ParamsError::Tools(_)
        | ParamsError::UserId(_)
        | ParamsError::NotNumber { .. }
        | ParamsError::NotList { .. }
        | ParamsError::NotAnObject
        | ParamsError::OutOfRange { .. }
        | ParamsError::NotBool { .. }
        | ParamsError::InvalidEffort { .. }
        | ParamsError::EffortUnsupported { .. }
        | ParamsError::Encode(_)
        | ParamsError::Decode(_) => "BadRequestError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: format!("litellm.{class}: {error}"),
    })
}

fn output_config_value(map: &IndexMap<String, Json>) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(map.get("output_config")).or_value_error()
}

async fn answer<Source, Transport>(
    step: &Step,
    context: &StepContext<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Value, Box<HelperError>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    match step {
        Step::MapOpenaiParams {
            model,
            non_default_params,
            optional_params,
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
            let mut mapped = optional_params.clone();
            if let Err(error) =
                map_openai_params(&request, non_default_params, &mut mapped, environment).await
            {
                return Err(helper_error(&error));
            }
            Ok(json!({
                "optional_params": serde_json::to_value(&mapped).or_value_error()?,
                "warning": context.warnings.first(),
            }))
        }
        Step::ApplyOutputConfig {
            model,
            data,
            optional_params,
            global_drop_params,
        } => {
            let request = OutputConfigRequest {
                model,
                provider: PROVIDER,
                global_drop: *global_drop_params,
            };
            let mut updated_data = data.clone();
            let mut updated_optional = optional_params.clone();
            if let Err(error) = apply_output_config(
                &request,
                &mut updated_data,
                &mut updated_optional,
                environment,
                context.catalogs,
            )
            .await
            {
                return Err(helper_error(&error));
            }
            Ok(json!({
                "data": {"output_config": output_config_value(&updated_data)?},
                "optional_params": {"output_config": output_config_value(&updated_optional)?},
                "warning": context.warnings.first(),
            }))
        }
    }
}
