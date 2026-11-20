mod fixtures;

use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_core::api::messages_chat_tools_schema::tools::ToolNameMapping;
use betterlite_core::api::messages_chat_transform::adapter::AnthropicAdapter;
use betterlite_core::api::messages_chat_transform::compose::translate_anthropic_to_openai;
use betterlite_core::api::messages_chat_transform::error::TransformError;
use betterlite_core::api::messages_chat_transform::settings::AdapterSettings;
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, PolyfillResult, UsageIteration,
};
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "input_params")]
    Params {
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
        response: Box<ResponseRecord>,
        #[serde(default)]
        tool_name_mapping: Option<Box<ToolNameMapping>>,
        #[serde(default)]
        polyfill_result: Option<Box<PolyfillInput>>,
    },
}

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

#[derive(Serialize)]
struct WrappedAnswer {
    request: Object,
    tool_name_mapping: Option<ToolNameMapping>,
    kwargs: Object,
}

#[derive(Serialize)]
struct ComposeAnswer {
    request: Object,
    tool_name_mapping: ToolNameMapping,
    anthropic_request: Object,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Params {
                kwargs,
                custom_llm_provider,
            } => input_params_answer(kwargs, custom_llm_provider.as_deref()),
            Input::Convenience {
                kwargs,
                custom_llm_provider,
            } => convenience_answer(kwargs, custom_llm_provider.as_deref()),
            Input::Compose {
                request,
                custom_llm_provider,
                preserve_midturn_system,
            } => compose_answer(
                request,
                custom_llm_provider.as_deref(),
                *preserve_midturn_system,
            ),
            Input::Response {
                response,
                tool_name_mapping,
                polyfill_result,
            } => response_answer(
                response,
                tool_name_mapping.as_deref(),
                polyfill_result.as_deref(),
            ),
        }
    }
}

fn build_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
}

fn input_params_answer(input_kwargs: &Object, custom_llm_provider: Option<&str>) -> HelperResult {
    let runtime = build_runtime()?;
    let answer = wrapped_answer(input_kwargs, custom_llm_provider, true, &runtime)?;
    serde_json::to_value(answer).map_err(|error| failure("SerializationError", error.to_string()))
}

fn convenience_answer(input_kwargs: &Object, custom_llm_provider: Option<&str>) -> HelperResult {
    let runtime = build_runtime()?;
    let answer = wrapped_answer(input_kwargs, custom_llm_provider, false, &runtime)?;
    serde_json::to_value(answer).map_err(|error| failure("SerializationError", error.to_string()))
}

fn wrapped_answer(
    input_kwargs: &Object,
    custom_llm_provider: Option<&str>,
    with_tool_mapping: bool,
    runtime: &Runtime,
) -> Result<WrappedAnswer, Box<HelperError>> {
    let mut kwargs = input_kwargs.clone();
    let declared = fixtures::Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.custom_llm_provider = custom_llm_provider;
    let mut capabilities = fixtures::Unsupported;
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
            settings,
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
) -> HelperResult {
    let runtime = build_runtime()?;
    let mut anthropic_request = source.clone();
    let declared = fixtures::Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.custom_llm_provider = custom_llm_provider;
    settings.preserve_midturn_system = preserve_midturn_system;
    let mut capabilities = fixtures::Unsupported;
    let (request, tool_name_mapping) = runtime
        .block_on(translate_anthropic_to_openai(
            &mut anthropic_request,
            &settings,
            &mut capabilities,
        ))
        .map_err(|error| transform_failure(&error))?;
    serde_json::to_value(ComposeAnswer {
        request,
        tool_name_mapping,
        anthropic_request,
    })
    .map_err(|error| failure("SerializationError", error.to_string()))
}

fn response_answer(
    response: &ResponseRecord,
    tool_name_mapping: Option<&ToolNameMapping>,
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
