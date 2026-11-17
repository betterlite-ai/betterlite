use std::fmt::Display;

use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_bedrock::converse_tools::additional_properties::add_additional_properties;
use betterlite_bedrock::converse_tools::computer_use::{
    is_computer_use_tool_used, separate_computer_use_tools, transform_computer_use_tools,
};
use betterlite_bedrock::converse_tools::json_mode::{
    apply_tool_call_transformation, create_json_tool_call,
};
use betterlite_bedrock::converse_tools::media_types::{
    SUPPORTED_DOCUMENT_TYPES, SUPPORTED_IMAGE_TYPES, SUPPORTED_VIDEO_TYPES,
};
use betterlite_bedrock::converse_tools::response_format::{
    ResponseFormatRequest, create_output_config, translate_response_format,
};
use betterlite_bedrock::converse_tools::tool_choice::map_tool_choice_values;
use betterlite_bedrock::converse_tools::tool_spec::BedrockToolSpec;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use serde_json::json;
use tokio::runtime::Builder;

use super::fixture::{CaseCapabilities, Fixture};
use super::input::{
    ApplyToolsInput, DetectorInput, Input, SeparateInput, ToolChoiceInput, ToolSpecInput,
    TransformInput, TranslateInput,
};

fn failed<E>(error: E, status: Option<u16>) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockConverseTools".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn tool_choice(input: &ToolChoiceInput) -> HelperResult {
    let fixture = Fixture::load(&input.model_cost).map_err(|error| failed(&error, None))?;
    let catalogs = fixture.catalogs();
    let drop = DropParams {
        global: input.global_drop,
        call: input.call_drop,
    };
    let mut names = ToolNameMap::new();
    let block = map_tool_choice_values(
        &input.model,
        &input.tool_choice,
        drop,
        &catalogs,
        &mut names,
    )
    .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "block": block }))
}

fn detector(input: &DetectorInput) -> HelperResult {
    let used = is_computer_use_tool_used(input.tools.as_deref())
        .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "used": used }))
}

fn separate(input: &SeparateInput) -> HelperResult {
    let (computer, regular) = separate_computer_use_tools(&input.tools)
        .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "computer": computer, "regular": regular }))
}

fn transform(input: &TransformInput) -> HelperResult {
    let tools = transform_computer_use_tools(&input.tools)
        .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "tools": tools }))
}

fn apply_tools(input: &ApplyToolsInput) -> HelperResult {
    let mut params = input.optional_params.clone();
    apply_tool_call_transformation(&input.tools, &input.model, &input.stream, &mut params)
        .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "params": params }))
}

fn translate(input: &TranslateInput) -> HelperResult {
    let fixture = Fixture::load(&input.model_cost).map_err(|error| failed(&error, None))?;
    let catalogs = fixture.catalogs();
    let mut capabilities = CaseCapabilities { fixture: &fixture };
    let request = ResponseFormatRequest {
        model: &input.model,
        is_thinking_enabled: input.is_thinking_enabled,
        stream: &input.stream,
        catalogs: &catalogs,
    };
    let mut params = input.optional_params.clone();
    Builder::new_current_thread()
        .build()
        .map_err(|error| failed(&error, None))?
        .block_on(translate_response_format(
            &input.value,
            &request,
            &mut params,
            &mut capabilities,
        ))
        .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "params": params }))
}

fn tool_spec(input: &ToolSpecInput) -> HelperResult {
    let block = BedrockToolSpec {
        name: &input.name,
        description: &input.description,
        parameters: &input.parameters,
        strict: input.strict,
        supports_strict_tools: input.supports_strict_tools,
    }
    .into_block()
    .map_err(|error| failed(&error, error.status_code()))?;
    Ok(json!({ "block": block }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ToolChoice(call) => tool_choice(call),
        Input::Detector(call) => detector(call),
        Input::Separate(call) => separate(call),
        Input::Transform(call) => transform(call),
        Input::JsonTool(call) => Ok(json!({
            "tool": create_json_tool_call(call.json_schema.as_ref(), call.description.as_deref()),
        })),
        Input::ApplyTools(call) => apply_tools(call),
        Input::Translate(call) => translate(call),
        Input::Additional(call) => Ok(json!({ "schema": add_additional_properties(&call.schema) })),
        Input::OutputConfig(call) => Ok(json!({
            "config": create_output_config(
                call.json_schema.as_ref(),
                call.name.as_deref(),
                call.description.as_deref(),
            ),
        })),
        Input::ToolSpec(call) => tool_spec(call),
        Input::MediaTypes => Ok(json!({
            "image": SUPPORTED_IMAGE_TYPES,
            "document": SUPPORTED_DOCUMENT_TYPES,
            "video": SUPPORTED_VIDEO_TYPES,
        })),
    }
}
