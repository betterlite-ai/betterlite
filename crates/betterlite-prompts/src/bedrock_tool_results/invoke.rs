use crate::arguments::concatenated::split_concatenated_json;
use crate::arguments::json::{Json, Object};

use super::error::BedrockToolResultsError;
use super::ids::sanitize_bedrock_tool_use_id;
use super::types::{ToolInvokeBlock, ToolUseBlock};

const FUNCTION: &str = "function";
const ID: &str = "id";
const NAME: &str = "name";
const ARGUMENTS: &str = "arguments";
const CACHE_CONTROL: &str = "cache_control";

// B602 owns AmazonConverseConfig.get_cache_point_block; its model-capability check and TTL stay behind this seam
pub trait CachePoints {
    fn cache_point_block(&self, cache_control: &Json, model: Option<&str>) -> Option<Object>;
}

enum ParsedArguments {
    Object(Object),
    NotAnObject,
    Concatenated(Vec<Object>),
    Unparsable,
}

// LiteLLM's make_valid_bedrock_tool_name, whose reverse-name map belongs to the caller
fn valid_tool_name(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut valid: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if !valid.chars().next().is_some_and(char::is_alphabetic) {
        valid.insert(0, 'a');
    }
    valid
}

fn parse_arguments(arguments: &str) -> ParsedArguments {
    if arguments.trim().is_empty() {
        return ParsedArguments::Object(Object::new());
    }
    match serde_json::from_str::<Json>(arguments) {
        Ok(Json::Object(input)) => ParsedArguments::Object(input),
        Ok(_) => ParsedArguments::NotAnObject,
        Err(_) => {
            let objects = split_concatenated_json(arguments);
            if objects.is_empty() {
                ParsedArguments::Unparsable
            } else {
                ParsedArguments::Concatenated(objects)
            }
        }
    }
}

fn split_tool_use_id(tool_id: &str, index: usize) -> String {
    if index == 0 {
        sanitize_bedrock_tool_use_id(tool_id)
    } else {
        sanitize_bedrock_tool_use_id(&format!("{tool_id}_{index}"))
    }
}

struct InvokeContext<'call, C> {
    model: Option<&'call str>,
    cache_points: &'call C,
}

fn append_cache_point<C>(
    blocks: &mut Vec<ToolInvokeBlock>,
    tool: &Object,
    context: &InvokeContext<'_, C>,
) where
    C: CachePoints,
{
    if let Some(cache_control) = tool.get(CACHE_CONTROL)
        && !matches!(cache_control, Json::Null)
        && let Some(cache_point) = context
            .cache_points
            .cache_point_block(cache_control, context.model)
    {
        blocks.push(ToolInvokeBlock::CachePoint { cache_point });
    }
}

fn append_tool_call_blocks<C>(
    blocks: &mut Vec<ToolInvokeBlock>,
    tool: &Object,
    function: &Object,
    index: usize,
    context: &InvokeContext<'_, C>,
) -> Result<(), BedrockToolResultsError>
where
    C: CachePoints,
{
    let tool_id = match tool.get(ID) {
        None => return Err(BedrockToolResultsError::ToolCallWithoutId { index }),
        Some(Json::Text(id)) => id.as_str(),
        Some(_) => return Err(BedrockToolResultsError::ToolCallIdNotText { index }),
    };
    let raw_name = match function.get(NAME) {
        None | Some(Json::Null) => "",
        Some(Json::Text(name)) => name.as_str(),
        Some(_) => return Err(BedrockToolResultsError::FunctionNameNotText { index }),
    };
    let name = valid_tool_name(raw_name);
    // LiteLLM's `not arguments` sends every falsy value down the empty-input branch before any parse
    let arguments = match function.get(ARGUMENTS) {
        None => "",
        Some(value) if !value.is_truthy() => "",
        Some(Json::Text(arguments)) => arguments.as_str(),
        Some(_) => return Err(BedrockToolResultsError::ArgumentsNotText { index }),
    };
    let created = match parse_arguments(arguments) {
        ParsedArguments::Concatenated(objects) => objects
            .into_iter()
            .enumerate()
            .map(|(split_index, input)| (input, split_tool_use_id(tool_id, split_index)))
            .collect::<Vec<(Object, String)>>(),
        ParsedArguments::Object(input) => vec![(input, sanitize_bedrock_tool_use_id(tool_id))],
        ParsedArguments::NotAnObject | ParsedArguments::Unparsable => {
            vec![(Object::new(), sanitize_bedrock_tool_use_id(tool_id))]
        }
    };
    for (input, tool_use_id) in created {
        blocks.push(ToolInvokeBlock::ToolUse {
            tool_use: ToolUseBlock {
                input,
                name: name.clone(),
                tool_use_id,
            },
        });
    }
    append_cache_point(blocks, tool, context);
    Ok(())
}

pub fn tool_call_invoke_blocks<C>(
    tool_calls: &[Json],
    model: Option<&str>,
    cache_points: &C,
) -> Result<Vec<ToolInvokeBlock>, BedrockToolResultsError>
where
    C: CachePoints,
{
    let mut blocks: Vec<ToolInvokeBlock> = Vec::new();
    let context = InvokeContext {
        model,
        cache_points,
    };
    for (index, tool) in tool_calls.iter().enumerate() {
        let Some(tool_fields) = tool.as_object() else {
            return Err(BedrockToolResultsError::ToolCallNotObject { index });
        };
        let Some(function) = tool_fields.get(FUNCTION) else {
            continue;
        };
        let Some(function_fields) = function.as_object() else {
            return Err(BedrockToolResultsError::FunctionNotObject { index });
        };
        append_tool_call_blocks(&mut blocks, tool_fields, function_fields, index, &context)?;
    }
    Ok(blocks)
}
