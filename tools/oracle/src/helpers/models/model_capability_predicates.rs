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
enum Core {
    Factory,
    DeclaredValue,
    Disabled,
    NativeStreaming,
    ResponseSchema,
}

const fn core_of(function: Function) -> Option<Core> {
    match function {
        Function::SupportsFactory => Some(Core::Factory),
        Function::DeclaredValueFactory => Some(Core::DeclaredValue),
        Function::IsExplicitlyDisabledFactory => Some(Core::Disabled),
        Function::SupportsNativeStreaming => Some(Core::NativeStreaming),
        Function::SupportsResponseSchema => Some(Core::ResponseSchema),
        Function::SupportsSystemMessages
        | Function::SupportsParallelCalls
        | Function::SupportsFunctionCalling
        | Function::SupportsToolChoice
        | Function::SupportsPromptCaching
        | Function::SupportsReasoning
        | Function::SupportsNoneReasoningEffort
        | Function::SupportsMidConversationSystem
        | Function::SupportsNativeStructuredOutput => None,
    }
}

const fn named_capability(function: Function) -> Option<NamedCapability> {
    match function {
        Function::SupportsSystemMessages => Some(NamedCapability::SystemMessages),
        Function::SupportsParallelCalls => Some(NamedCapability::ParallelCalls),
        Function::SupportsFunctionCalling => Some(NamedCapability::FunctionCalling),
        Function::SupportsToolChoice => Some(NamedCapability::ToolChoice),
        Function::SupportsPromptCaching => Some(NamedCapability::PromptCaching),
        Function::SupportsReasoning => Some(NamedCapability::Reasoning),
        Function::SupportsNoneReasoningEffort => Some(NamedCapability::NoneReasoningEffort),
        Function::SupportsMidConversationSystem => Some(NamedCapability::MidConversationSystem),
        Function::SupportsNativeStructuredOutput => {
            Some(NamedCapability::NativeStructuredOutput)
        }
        Function::SupportsFactory
        | Function::DeclaredValueFactory
        | Function::IsExplicitlyDisabledFactory
        | Function::SupportsNativeStreaming
        | Function::SupportsResponseSchema => None,
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
        let fixture = Fixture::load(&input.fixture).map_err(|error| failure(&error))?;
        let resolution = fixture.resolution();
        let mut keys = ModelCostKeys::default();
        let dynamic = NoDynamicMetadata;
        let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| failure(&FixtureError::Runtime { source }))?;
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

fn failure(error: &FixtureError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "FixtureError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn serialization_failure(error: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn missing_key(function: Function) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: format!("the {function:?} function needs a key"),
    })
}

// The three keyed factories read the same declaration; a step without one is a malformed call.



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
    let Some(core) = core_of(step.function) else {
        let capability =
            named_capability(step.function).ok_or_else(|| missing_key(step.function))?;
        return Ok(Value::Bool(
            environment.supports_named(&request, capability).await,
        ));
    };
    core_answer(core, step, &request, environment).await
}

async fn core_answer<Source, Transport>(
    core: Core,
    step: &Step,
    request: &CapabilityRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Value, Box<HelperError>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let declared = step.key.as_deref();
    let value = match core {
        Core::Factory => {
            let key = declared.ok_or_else(|| missing_key(step.function))?;
            Value::Bool(environment.supports_factory(request, key).await)
        }
        Core::DeclaredValue => {
            let key = declared.ok_or_else(|| missing_key(step.function))?;
            environment
                .declared_value_factory(request, key)
                .await
                .map_or(Value::Null, Value::String)
        }
        Core::Disabled => {
            let key = declared.ok_or_else(|| missing_key(step.function))?;
            Value::Bool(
                environment
                    .is_explicitly_disabled_factory(request, key)
                    .await,
            )
        }
        Core::NativeStreaming => {
            let flagged = environment.supports_native_streaming(request).await;
            serde_json::to_value(flagged).map_err(|error| serialization_failure(&error))?
        }
        Core::ResponseSchema => Value::Bool(environment.supports_response_schema(request).await),
    };
    Ok(value)
}
