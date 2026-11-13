mod fake;

use betterlite_core::api::messages_content_primitives::markers::{
    carry_cache_breakpoint, copy_cache_control, mid_system_supported, target_consumes_cache_control,
};
use betterlite_core::api::messages_content_primitives::signatures::{
    ToolCallFields, compaction_blocks, tool_call_signature, tool_use_signature,
};
use betterlite_core::api::messages_content_primitives::tool_results::{
    image_source_url, tool_result_content, tool_result_image_part, tool_result_part,
};
use betterlite_core::api::messages_content_primitives::tools::{
    ChatToolParam, TRANSLATABLE_ANTHROPIC_PARAMS, chat_tool_param, is_openai_function_tool,
    is_provider_native_tool_dict, is_web_search_tool, truncate_tool_name,
};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::to_value;

use crate::helpers::{Helper, HelperError, HelperResult};
use fake::FlaggedModels;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    TruncateToolName {
        name: String,
    },
    #[serde(rename = "is_provider_native_tool_dict")]
    NativeTool {
        tool: Object,
    },
    IsOpenaiFunctionTool {
        tool: Object,
    },
    IsWebSearchTool {
        tool: Object,
    },
    TranslatableParams,
    // A raw mapping carries no attributes, so only an attribute-bearing call reads signatures.
    ToolCallSignature {
        tool_call: ToolCallFields,
        attributes: bool,
    },
    ToolUseSignature {
        content: Object,
    },
    CompactionBlocks {
        fields: Object,
    },
    MidSystemSupported(MidSystem),
    ChatToolParam {
        function: Object,
        tool: Json,
    },
    CopyCacheControl(CopyCache),
    #[serde(rename = "carry_cache_breakpoint")]
    CarryBreakpoint {
        source: Object,
        target: Object,
    },
    ConsumesCacheControl {
        model: String,
    },
    ImageSourceUrl {
        image_source: Json,
    },
    #[serde(rename = "tool_result_image_part")]
    ImagePart {
        image_source: Json,
    },
    ToolResultPart {
        item: Json,
    },
    ToolResultContent {
        raw_content: Json,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::TruncateToolName { name } => to_value(truncate_tool_name(name)),
            Input::NativeTool { tool } => to_value(is_provider_native_tool_dict(tool)),
            Input::IsOpenaiFunctionTool { tool } => to_value(is_openai_function_tool(tool)),
            Input::IsWebSearchTool { tool } => to_value(is_web_search_tool(tool)),
            Input::TranslatableParams => to_value(TRANSLATABLE_ANTHROPIC_PARAMS),
            Input::ToolCallSignature {
                tool_call,
                attributes,
            } => to_value(attributes.then(|| tool_call_signature(tool_call)).flatten()),
            Input::ToolUseSignature { content } => to_value(tool_use_signature(content)),
            Input::CompactionBlocks { fields } => to_value(compaction_blocks(fields)),
            Input::MidSystemSupported(call) => to_value(call.supported()?),
            Input::ChatToolParam { function, tool } => to_value(tool_param(function, tool)?),
            Input::CopyCacheControl(call) => to_value(call.copied()),
            Input::CarryBreakpoint { source, target } => {
                to_value(carry_cache_breakpoint(source, target))
            }
            Input::ConsumesCacheControl { model } => to_value(target_consumes_cache_control(model)),
            Input::ImageSourceUrl { image_source } => to_value(image_source_url(image_source)),
            Input::ImagePart { image_source } => {
                to_value(tool_result_image_part(Some(image_source)))
            }
            Input::ToolResultPart { item } => to_value(tool_result_part(item)),
            Input::ToolResultContent { raw_content } => to_value(tool_result_content(raw_content)),
        };
        answer.map_err(|source| failure("SerializationError", None, source.to_string()))
    }
}

fn tool_param(function: &Object, tool: &Json) -> Result<ChatToolParam, Box<HelperError>> {
    match chat_tool_param(function.clone(), tool) {
        Ok(param) => Ok(param),
        Err(error) => Err(failure(
            "UnsupportedParamsError",
            error.status_code(),
            format!("litellm.UnsupportedParamsError: {error}"),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CopyCache {
    source: Object,
    target: Object,
    model: Option<String>,
}

impl CopyCache {
    fn copied(&self) -> Object {
        let mut copied = self.target.clone();
        copy_cache_control(&self.source, &mut copied, self.model.as_deref());
        copied
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct MidSystem {
    model: Option<String>,
    custom_llm_provider: Option<String>,
    supported: Vec<String>,
}

impl MidSystem {
    fn supported(&self) -> Result<bool, Box<HelperError>> {
        let mut predicates = FlaggedModels {
            models: self.supported.clone(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|source| failure("RuntimeError", None, source.to_string()))?;
        Ok(runtime.block_on(mid_system_supported(
            &mut predicates,
            self.model.as_deref(),
            self.custom_llm_provider.as_deref(),
        )))
    }
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
