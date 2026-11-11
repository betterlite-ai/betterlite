mod fixture;
mod tests;

use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest, NamedCapability,
};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::{
    DynamicMetadataSource, NoDynamicMetadata,
};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{HelperError, HelperResult};
use fixture::{Fixture, FixtureError};

pub(crate) struct Op;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Function {
    SupportsFactory,
    DeclaredValueFactory,
    IsExplicitlyDisabledFactory,
    SupportsNativeStreaming,
    SupportsResponseSchema,
    SupportsSystemMessages,
    #[serde(rename = "supports_parallel_function_calling")]
    SupportsParallelCalls,
    SupportsFunctionCalling,
    SupportsToolChoice,
    SupportsPromptCaching,
    SupportsReasoning,
    SupportsNoneReasoningEffort,
    SupportsMidConversationSystem,
    SupportsNativeStructuredOutput,
}

// The four shared readers and the factory itself; the nine named rows are factory calls with a fixed key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Factory,
    DeclaredValue,
    Disabled,
    NativeStreaming,
    ResponseSchema,
    Named(NamedCapability),
}

const fn call_of(function: Function) -> Call {
    match function {
        Function::SupportsFactory => Call::Factory,
        Function::DeclaredValueFactory => Call::DeclaredValue,
        Function::IsExplicitlyDisabledFactory => Call::Disabled,
        Function::SupportsNativeStreaming => Call::NativeStreaming,
        Function::SupportsResponseSchema => Call::ResponseSchema,
        Function::SupportsSystemMessages => Call::Named(NamedCapability::SystemMessages),
        Function::SupportsParallelCalls => Call::Named(NamedCapability::ParallelCalls),
        Function::SupportsFunctionCalling => Call::Named(NamedCapability::FunctionCalling),
        Function::SupportsToolChoice => Call::Named(NamedCapability::ToolChoice),
        Function::SupportsPromptCaching => Call::Named(NamedCapability::PromptCaching),
        Function::SupportsReasoning => Call::Named(NamedCapability::Reasoning),
        Function::SupportsNoneReasoningEffort => Call::Named(NamedCapability::NoneReasoningEffort),
        Function::SupportsMidConversationSystem => {
            Call::Named(NamedCapability::MidConversationSystem)
        }
        Function::SupportsNativeStructuredOutput => {
            Call::Named(NamedCapability::NativeStructuredOutput)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    function: Function,
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: fixture::FixtureInput,
    steps: Vec<Step>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::load(&input.fixture).map_err(|error| {
            Box::new(HelperError {
                class: String::from("FixtureError"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let resolution = fixture.resolution();
        let mut keys = ModelCostKeys::default();
        let dynamic = NoDynamicMetadata;
        let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| {
                Box::new(HelperError {
                    class: String::from("FixtureError"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: FixtureError::Runtime { source }.to_string(),
                })
            })?;
        let answers = runtime.block_on(async {
            let mut answers = Vec::with_capacity(input.steps.len());
            for step in &input.steps {
                answers.push(answer(step, &mut environment).await?);
            }
            Ok::<Vec<Value>, Box<HelperError>>(answers)
        })?;
        Ok(Value::Array(answers))
    }
}

// The three keyed readers read the same declaration; a step without one is a malformed call.
fn key_of(step: &Step) -> Result<&str, Box<HelperError>> {
    step.key.as_deref().ok_or_else(|| {
        Box::new(HelperError {
            class: String::from("ValueError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("the {:?} function needs a key", step.function),
        })
    })
}

async fn answer<Source, Transport>(
    step: &Step,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Value, Box<HelperError>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let request = CapabilityRequest {
        model: &step.model,
        custom_llm_provider: step.custom_llm_provider.as_deref(),
    };
    let value = match call_of(step.function) {
        Call::Factory => Value::Bool(environment.supports_factory(&request, key_of(step)?).await),
        Call::DeclaredValue => environment
            .declared_value_factory(&request, key_of(step)?)
            .await
            .map_or(Value::Null, Value::String),
        Call::Disabled => Value::Bool(
            environment
                .is_explicitly_disabled_factory(&request, key_of(step)?)
                .await,
        ),
        Call::NativeStreaming => {
            let flagged = environment.supports_native_streaming(&request).await;
            serde_json::to_value(flagged).map_err(|error| {
                Box::new(HelperError {
                    class: String::from("SerializationError"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?
        }
        Call::ResponseSchema => Value::Bool(environment.supports_response_schema(&request).await),
        Call::Named(capability) => Value::Bool(
            environment
                .supports_factory(&request, capability.key())
                .await,
        ),
    };
    Ok(value)
}
