use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_schema_shaping::{Raw, Settings};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::gemini_tools::assembly::map_tools;
use betterlite_vertex::gemini_tools::error::GeminiToolsError;
use betterlite_vertex::gemini_tools::options::{
    drop_mixed_search_tools, map_service_tier, map_tool_choice, map_web_search_options,
};
use betterlite_vertex::gemini_tools::records::{Tool, ToolConfig};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;
pub(crate) struct Op;
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    call: Call,
    settings: BTreeMap<String, String>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Call {
    tools: Vec<Object>,
    functions: Vec<Object>,
    web_search_options: Option<Object>,
    tool_choice: Option<Json>,
    service_tier: Option<String>,
    server_side_invocations: bool,
    tool_config: Option<ToolConfig>,
}
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let settings = Raw::default().convert(&input.settings).or_config_error()?;
        measure(&input.call, &settings)
    }
}
struct Sources {
    tools: Option<Vec<Tool>>,
    tool_config: Option<ToolConfig>,
    warnings: usize,
}
fn map_sources(call: &Call, settings: &Settings) -> Result<Sources, Box<HelperError>> {
    // LiteLLM maps the tools list and the functions list in separate calls, then appends one result to the other
    let mut tools: Option<Vec<Tool>> = None;
    let mut tool_config = call.tool_config.clone();
    let mut warnings = 0_usize;
    let lists = [&call.tools, &call.functions]
        .into_iter()
        .filter(|source| !source.is_empty());
    for source in lists {
        let mapped = map_tools(source, call.server_side_invocations, tool_config, settings)
            .or_value_error()?;
        warnings = warnings.saturating_add(mapped.warnings);
        tool_config = mapped.tool_config;
        tools.get_or_insert_with(Vec::new).extend(mapped.tools);
    }
    Ok(Sources {
        tools,
        tool_config,
        warnings,
    })
}
fn measure(call: &Call, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let mut sources = map_sources(call, settings)?;
    let tool_choice = call
        .tool_choice
        .as_ref()
        .map(map_tool_choice)
        .transpose()
        .map_err(|error| choice_failure(&error))?;
    if let Some(options) = &call.web_search_options {
        sources
            .tools
            .get_or_insert_with(Vec::new)
            .push(map_web_search_options(options));
    }
    if let Some(existing) = &mut sources.tools {
        sources.warnings = sources
            .warnings
            .saturating_add(usize::from(drop_mixed_search_tools(
                existing,
                call.server_side_invocations,
            )));
    }
    let service_tier = call.service_tier.as_deref().map(map_service_tier);
    Ok(json!({
        "tools": sources.tools,
        "tool_choice": tool_choice,
        "toolConfig": sources.tool_config,
        "service_tier": service_tier,
        "warnings": sources.warnings,
    }))
}
fn choice_failure(error: &GeminiToolsError) -> Box<HelperError> {
    let unsupported = matches!(error, GeminiToolsError::UnsupportedToolChoice);
    let class = if unsupported {
        "UnsupportedParamsError"
    } else {
        "ValueError"
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: unsupported.then_some(400),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
