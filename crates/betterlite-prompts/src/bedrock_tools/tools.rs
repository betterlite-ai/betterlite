use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::cache_point::add_cache_point_tool_block;
use crate::bedrock_tools::capabilities::BedrockToolCapabilities;
use crate::bedrock_tools::error::BedrockToolError;
use crate::bedrock_tools::name::{ToolNameMap, make_valid_bedrock_tool_name};
use crate::bedrock_tools::native::is_bedrock_tool_block;
use crate::bedrock_tools::schema::{ExpansionBudget, parameter_schema};
use crate::definitions::legacy::DEFS_MAX_INLINED_BYTES;

const FUNCTION: &str = "function";
const INPUT_SCHEMA: &str = "input_schema";
const UNNAMED_PREFIX: &str = "litellm_unnamed_tool_";

pub struct BedrockTools {
    pub blocks: Vec<Json>,
    pub names: ToolNameMap,
}

struct ToolSource<'fields> {
    raw_name: &'fields str,
    description: Option<&'fields str>,
    parameters: Option<&'fields Json>,
    strict: Option<&'fields Json>,
}

// A native block is copied untouched, a Responses built-in without a tool body is dropped, and every other tool becomes a toolSpec.
pub fn bedrock_tools(
    tools: &[Json],
    capabilities: BedrockToolCapabilities,
) -> Result<BedrockTools, BedrockToolError> {
    let mut names = ToolNameMap::new();
    let mut budget = ExpansionBudget::new(DEFS_MAX_INLINED_BYTES);
    let mut blocks = Vec::with_capacity(tools.len());
    for (index, tool) in tools.iter().enumerate() {
        if is_bedrock_tool_block(tool) {
            blocks.push(tool.clone());
            continue;
        }
        let Json::Object(fields) = tool else {
            return Err(BedrockToolError::ToolNotObject { index });
        };
        let Some(source) = tool_source(fields)? else {
            continue;
        };
        blocks.push(tool_spec(
            index,
            &source,
            capabilities,
            &mut names,
            &mut budget,
        )?);
        if let Some(block) = add_cache_point_tool_block(fields, capabilities.cache_points)? {
            blocks.push(block);
        }
    }
    Ok(BedrockTools { blocks, names })
}

fn tool_source(fields: &Object) -> Result<Option<ToolSource<'_>>, BedrockToolError> {
    match fields.get(FUNCTION) {
        Some(Json::Object(function)) => Ok(Some(ToolSource {
            raw_name: text_or_empty(function.get("name"))?,
            description: optional_text(function.get("description"))?,
            parameters: function.get("parameters"),
            strict: function.get("strict"),
        })),
        Some(_) => Err(BedrockToolError::WrongType { field: FUNCTION }),
        None => match fields.get(INPUT_SCHEMA) {
            Some(_) => Ok(Some(ToolSource {
                raw_name: text_or_empty(fields.get("name"))?,
                description: optional_text(fields.get("description"))?,
                parameters: fields.get(INPUT_SCHEMA),
                strict: None,
            })),
            None => Ok(None),
        },
    }
}

fn text_or_empty(value: Option<&Json>) -> Result<&str, BedrockToolError> {
    match value {
        None | Some(Json::Null) => Ok(""),
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(BedrockToolError::WrongType { field: "name" }),
    }
}

fn optional_text(value: Option<&Json>) -> Result<Option<&str>, BedrockToolError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(BedrockToolError::WrongType {
            field: "description",
        }),
    }
}

fn tool_spec(
    index: usize,
    source: &ToolSource<'_>,
    capabilities: BedrockToolCapabilities,
    names: &mut ToolNameMap,
    budget: &mut ExpansionBudget,
) -> Result<Json, BedrockToolError> {
    let raw_name = if source.raw_name.trim().is_empty() {
        format!("{UNNAMED_PREFIX}{index}")
    } else {
        source.raw_name.to_owned()
    };
    let name = make_valid_bedrock_tool_name(&raw_name, names);
    let description = match source.description {
        Some(text) if !text.is_empty() => text.to_owned(),
        Some(_) | None => name.clone(),
    };
    let parameters = match source.parameters {
        None | Some(Json::Null) => Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("object".to_owned())),
            ("properties".to_owned(), Json::Object(Object::new())),
        ])),
        Some(value) => value.clone(),
    };
    let schema = parameter_schema(&parameters, capabilities, budget)?;
    let strict = match source.strict {
        None | Some(Json::Null | Json::Bool(false)) => false,
        Some(Json::Bool(true)) => true,
        Some(_) => return Err(BedrockToolError::WrongType { field: "strict" }),
    };
    let mut spec = Object::new();
    spec.insert(
        "inputSchema".to_owned(),
        Json::Object(Object::from_iter([("json".to_owned(), schema)])),
    );
    spec.insert("name".to_owned(), Json::Text(name));
    spec.insert("description".to_owned(), Json::Text(description));
    if capabilities.supports_strict_tools && strict {
        spec.insert("strict".to_owned(), Json::Bool(true));
    }
    Ok(Json::Object(Object::from_iter([(
        "toolSpec".to_owned(),
        Json::Object(spec),
    )])))
}
