use betterlite_prompts::arguments::json::{Json, Object};

use super::output_format::{OutputFormatError, output_format_to_openai};
use super::tool_choice::{ToolChoiceError, tool_choice_to_openai};
use super::tools::{ToolNameMapping, ToolsError, tools_to_openai};
use crate::api::messages_content_primitives::tools::is_web_search_tool;

pub fn translate_tool_choice(request: &Object, kwargs: &mut Object) -> Result<(), ToolChoiceError> {
    let Some(tool_choice) = request
        .get("tool_choice")
        .filter(|choice| choice.is_truthy())
    else {
        return Ok(());
    };
    let translated = tool_choice_to_openai(tool_choice)?;
    kwargs.insert("tool_choice".to_owned(), translated);
    Ok(())
}

pub fn translate_tools(
    request: &Object,
    kwargs: &mut Object,
) -> Result<ToolNameMapping, ToolsError> {
    let Some(entries) = request.get("tools").filter(|tools| tools.is_truthy()) else {
        return Ok(ToolNameMapping::new());
    };
    let Json::List(tools) = entries else {
        return Err(ToolsError::NotAList);
    };
    let mut has_web_search = false;
    let mut regular = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        let Some(fields) = tool.as_object() else {
            return Err(ToolsError::NotAnObject { index });
        };
        if is_web_search_tool(fields) {
            has_web_search = true;
        } else {
            regular.push(tool.clone());
        }
    }
    if has_web_search {
        kwargs.insert("web_search_options".to_owned(), Json::Object(Object::new()));
    }
    if regular.is_empty() {
        return Ok(ToolNameMapping::new());
    }
    let model = match kwargs.get("model") {
        None | Some(Json::Null) => None,
        Some(Json::Text(model)) => Some(model.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ToolsError::ModelNotText);
        }
    };
    let translation = tools_to_openai(&regular, model)?;
    kwargs.insert("tools".to_owned(), Json::List(translation.tools));
    Ok(translation.name_mapping)
}

pub fn translate_output_format(
    request: &Object,
    kwargs: &mut Object,
) -> Result<(), OutputFormatError> {
    let selected = request
        .get("output_format")
        .filter(|format| format.is_truthy())
        .or_else(|| {
            request
                .get("output_config")
                .and_then(Json::as_object)
                .and_then(|config| config.get("format"))
                .filter(|format| format.is_truthy())
        });
    let Some(output_format) = selected else {
        return Ok(());
    };
    if let Some(response_format) = output_format_to_openai(output_format)? {
        kwargs.insert("response_format".to_owned(), response_format);
    }
    Ok(())
}
