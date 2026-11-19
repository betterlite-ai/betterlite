use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_core::api::messages_chat_transform::adapter::AnthropicAdapter;
use betterlite_core::api::messages_chat_transform::compose::translate_anthropic_to_openai;
use betterlite_core::api::messages_chat_transform::error::TransformError;
use betterlite_core::api::messages_chat_transform::settings::AdapterSettings;
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, PolyfillResult, UsageIteration,
};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    InputParams {
        kwargs: Object,
        #[serde(default)]
        custom_llm_provider: Option<String>,
    },
    Convenience {
        kwargs: Object,
        #[serde(default)]
        custom_llm_provider: Option<String>,
    },
    Compose {
        request: Object,
        #[serde(default)]
        custom_llm_provider: Option<String>,
        #[serde(default)]
        preserve_midturn_system: bool,
    },
    Response {
        response: ResponseRecord,
        #[serde(default)]
        tool_name_mapping: Option<Objectless>,
        #[serde(default)]
        polyfill_result: Option<PolyfillInput>,
    },
}

pub(crate) type Objectless = indexmap::IndexMap<String, String>;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolyfillInput {
    #[serde(default)]
    applied_edits: Vec<AppliedEdit>,
    #[serde(default)]
    compaction_block: Option<CompactionBlock>,
    #[serde(default)]
    iterations_usage: Option<Vec<UsageIteration>>,
}

impl PolyfillInput {
    fn result(&self) -> PolyfillResult {
        PolyfillResult {
            messages: Vec::new(),
            system: None,
            applied_edits: self.applied_edits.clone(),
            compaction_block: self.compaction_block.clone(),
            iterations_usage: self.iterations_usage.clone(),
        }
    }
}

// The provider's declared parameters are unknown to a case, so every option step reads them as absent.
struct Declared;

impl SupportedParameterSource for Declared {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

// LiteLLM reads capabilities from its pinned model map, so a case never reaches a real lookup here.
struct Unsupported;

impl CapabilityPredicates for Unsupported {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = DecisionJson> + Send {
        ready(DecisionJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

#[derive(Serialize)]
struct WrappedAnswer {
    request: Object,
    tool_name_mapping: Option<Objectless>,
    kwargs: Object,
}

#[derive(Serialize)]
struct ComposeAnswer {
    request: Object,
    tool_name_mapping: Objectless,
    anthropic_request: Object,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        match input {
            Input::InputParams {
                kwargs,
                custom_llm_provider,
            } => serialized(wrapped_answer(
                kwargs,
                custom_llm_provider.as_deref(),
                true,
                &runtime,
            )?),
            Input::Convenience {
                kwargs,
                custom_llm_provider,
            } => serialized(wrapped_answer(
                kwargs,
                custom_llm_provider.as_deref(),
                false,
                &runtime,
            )?),
            Input::Compose {
                request,
                custom_llm_provider,
                preserve_midturn_system,
            } => serialized(compose_answer(
                request,
                custom_llm_provider.as_deref(),
                *preserve_midturn_system,
                &runtime,
            )?),
            Input::Response {
                response,
                tool_name_mapping,
                polyfill_result,
            } => response_answer(
                response,
                tool_name_mapping.as_ref(),
                polyfill_result.as_ref(),
            ),
        }
    }
}

fn serialized<T: Serialize>(answer: T) -> Result<serde_json::Value, Box<HelperError>> {
    serde_json::to_value(answer).map_err(|error| failure("SerializationError", error.to_string()))
}

fn wrapped_answer(
    input_kwargs: &Object,
    custom_llm_provider: Option<&str>,
    with_tool_mapping: bool,
    runtime: &Runtime,
) -> Result<WrappedAnswer, Box<HelperError>> {
    let mut kwargs = input_kwargs.clone();
    let declared = Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.custom_llm_provider = custom_llm_provider;
    let mut capabilities = Unsupported;
    let adapter = AnthropicAdapter::new();
    if with_tool_mapping {
        let (request, tool_name_mapping) = runtime
            .block_on(adapter.translate_completion_input_params_with_tool_mapping(
                &mut kwargs,
                &settings,
                &mut capabilities,
            ))
            .map_err(|error| transform_failure(&error))?;
        return Ok(WrappedAnswer {
            request,
            tool_name_mapping: Some(tool_name_mapping),
            kwargs,
        });
    }
    let request = runtime
        .block_on(adapter.translate_completion_input_params(
            &mut kwargs,
            &settings,
            &mut capabilities,
        ))
        .map_err(|error| transform_failure(&error))?;
    Ok(WrappedAnswer {
        request,
        tool_name_mapping: None,
        kwargs,
    })
}

fn compose_answer(
    source: &Object,
    custom_llm_provider: Option<&str>,
    preserve_midturn_system: bool,
    runtime: &Runtime,
) -> Result<ComposeAnswer, Box<HelperError>> {
    let mut anthropic_request = source.clone();
    let declared = Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.custom_llm_provider = custom_llm_provider;
    settings.preserve_midturn_system = preserve_midturn_system;
    let mut capabilities = Unsupported;
    let (request, tool_name_mapping) = runtime
        .block_on(translate_anthropic_to_openai(
            &mut anthropic_request,
            &settings,
            &mut capabilities,
        ))
        .map_err(|error| transform_failure(&error))?;
    Ok(ComposeAnswer {
        request,
        tool_name_mapping,
        anthropic_request,
    })
}

fn response_answer(
    response: &ResponseRecord,
    tool_name_mapping: Option<&Objectless>,
    polyfill_result: Option<&PolyfillInput>,
) -> Result<serde_json::Value, Box<HelperError>> {
    let polyfill = polyfill_result.map(PolyfillInput::result);
    let translated = AnthropicAdapter::new()
        .translate_completion_output_params(response, tool_name_mapping, polyfill.as_ref())
        .map_err(|error| failure("ValueError", error.to_string()))?;
    serde_json::to_value(&translated)
        .map_err(|error| failure("SerializationError", error.to_string()))
}

// LiteLLM raises KeyError for a popped key and ValueError for an empty one; the rest are typed refusals.
fn transform_failure(error: &TransformError) -> Box<HelperError> {
    let class = match error {
        TransformError::MissingKey { .. } => "KeyError",
        TransformError::ModelRequired
        | TransformError::MessagesRequired
        | TransformError::ModelNotText
        | TransformError::MessagesNotList => "ValueError",
        TransformError::Content(_)
        | TransformError::Options(_)
        | TransformError::ToolChoice(_)
        | TransformError::Tools(_)
        | TransformError::OutputFormat(_) => "TransformError",
    };
    failure(class, error.to_string())
}

// A case compares the error's class, status and fields; nothing here carries an HTTP status.
fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
