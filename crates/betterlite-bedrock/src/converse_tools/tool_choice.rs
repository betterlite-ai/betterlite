use betterlite_anthropic::anthropic_capability_catalog::forced_tool::forced_tool_use_downgraded;
use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_prompts::bedrock_tools::name::{ToolNameMap, make_valid_bedrock_tool_name};
use indexmap::IndexMap;

use crate::converse_tools::error::ConverseToolsError;

pub type ToolChoiceValuesBlock = IndexMap<String, Json>;

pub fn map_tool_choice_values(
    model: &str,
    tool_choice: &Json,
    drop: DropParams,
    catalogs: &CatalogSources<'_>,
    names: &mut ToolNameMap,
) -> Result<Option<ToolChoiceValuesBlock>, ConverseToolsError> {
    match tool_choice {
        Json::Text(text) if text == "none" => none_choice(drop).map(|()| None),
        Json::Text(text) if text == "required" => required_choice(model, drop, catalogs).map(Some),
        Json::Text(text) if text == "auto" => Ok(Some(choice("auto"))),
        Json::Object(fields) => {
            if forced_tool_use_downgraded(model, drop, catalogs)? {
                return Ok(Some(choice("auto")));
            }
            Ok(Some(specific_tool_choice(name_json(
                tool_choice_name(fields)?,
                names,
            )?)))
        }
        Json::Text(text) => Err(ConverseToolsError::UnsupportedChoice {
            value: text.clone(),
        }),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(ConverseToolsError::UnsupportedChoice {
                value: dumps(tool_choice),
            })
        }
    }
}

const fn none_choice(drop: DropParams) -> Result<(), ConverseToolsError> {
    if drop.global || drop.call {
        return Ok(());
    }
    Err(ConverseToolsError::UnsupportedNone)
}

fn required_choice(
    model: &str,
    drop: DropParams,
    catalogs: &CatalogSources<'_>,
) -> Result<ToolChoiceValuesBlock, ConverseToolsError> {
    if forced_tool_use_downgraded(model, drop, catalogs)? {
        return Ok(choice("auto"));
    }
    Ok(choice("any"))
}

#[must_use]
pub fn specific_tool_choice(name: Json) -> ToolChoiceValuesBlock {
    let mut tool = IndexMap::new();
    tool.insert("name".to_owned(), name);
    let mut block = IndexMap::new();
    block.insert("tool".to_owned(), Json::Object(tool));
    block
}

fn choice(kind: &str) -> ToolChoiceValuesBlock {
    let mut block = IndexMap::new();
    block.insert(kind.to_owned(), Json::Object(IndexMap::new()));
    block
}

fn tool_choice_name(fields: &IndexMap<String, Json>) -> Result<Option<&Json>, ConverseToolsError> {
    let Some(raw) = fields.get("function") else {
        return Ok(None);
    };
    let Json::Object(function) = raw else {
        return Err(ConverseToolsError::ToolChoiceFunction);
    };
    Ok(function.get("name"))
}

// LiteLLM hands the raw name to the sanitizer, so absent sanitizes "" and an explicit null stays null.
fn name_json(name: Option<&Json>, names: &mut ToolNameMap) -> Result<Json, ConverseToolsError> {
    match name {
        None => Ok(Json::Text(make_valid_bedrock_tool_name("", names))),
        Some(Json::Null) => Ok(Json::Null),
        Some(Json::Text(text)) => Ok(Json::Text(make_valid_bedrock_tool_name(text, names))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseToolsError::ToolChoiceName)
        }
    }
}
