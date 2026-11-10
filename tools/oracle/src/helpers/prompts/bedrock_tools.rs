use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_tools::cache_point::add_cache_point_tool_block;
use betterlite_prompts::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use betterlite_prompts::bedrock_tools::error::BedrockToolError;
use betterlite_prompts::bedrock_tools::name::{ToolNameMap, make_valid_bedrock_tool_name};
use betterlite_prompts::bedrock_tools::native::is_bedrock_tool_block;
use betterlite_prompts::bedrock_tools::tools::bedrock_tools;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

// The Python adapter reads `model` beside each call and resolves the same capability flags from LiteLLM's price map.
#[derive(Debug, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidNames {
        names: Vec<String>,
    },
    IsNative {
        tools: Vec<Json>,
    },
    CachePoint {
        scenarios: Vec<CachePointScenario>,
    },
    Tools {
        tools: Vec<Json>,
        capability: CapabilityInput,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct CachePointScenario {
    tool: Object,
    capability: CapabilityInput,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapabilityInput {
    cache_points: CachePointSetting,
    supports_strict_tools: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CachePointSetting {
    Unsupported,
    ShortTtl,
    ExtendedTtl,
}

impl CapabilityInput {
    const fn resolve(&self) -> BedrockToolCapabilities {
        let cache_points = match self.cache_points {
            CachePointSetting::Unsupported => CachePointSupport::Unsupported,
            CachePointSetting::ShortTtl => CachePointSupport::ShortTtl,
            CachePointSetting::ExtendedTtl => CachePointSupport::ExtendedTtl,
        };
        BedrockToolCapabilities {
            cache_points,
            supports_strict_tools: self.supports_strict_tools,
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ValidNames { names } => Ok(valid_names(names)),
            Input::IsNative { tools } => Ok(is_native(tools)),
            Input::CachePoint { scenarios } => cache_points(scenarios),
            Input::Tools { tools, capability } => tools_value(tools, capability.resolve()),
        }
    }
}

fn valid_names(names: &[String]) -> Value {
    let mut recorded = ToolNameMap::new();
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        let valid = make_valid_bedrock_tool_name(name, &mut recorded);
        let reverse = recorded
            .get(&valid)
            .map_or_else(|| valid.clone(), String::clone);
        entries.push(json!({"input": name, "valid": valid, "reverse": reverse}));
    }
    Value::Array(entries)
}

fn is_native(tools: &[Json]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| Value::Bool(is_bedrock_tool_block(tool)))
            .collect(),
    )
}

fn cache_points(scenarios: &[CachePointScenario]) -> HelperResult {
    let mut blocks = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let block =
            add_cache_point_tool_block(&scenario.tool, scenario.capability.resolve().cache_points)
                .map_err(|error| tool_failure(&error))?;
        blocks.push(match block {
            Some(value) => json_value(&value)?,
            None => Value::Null,
        });
    }
    Ok(Value::Array(blocks))
}

fn tools_value(tools: &[Json], capabilities: BedrockToolCapabilities) -> HelperResult {
    let converted = bedrock_tools(tools, capabilities).map_err(|error| tool_failure(&error))?;
    let blocks = converted
        .blocks
        .iter()
        .map(json_value)
        .collect::<Result<Vec<_>, _>>()?;
    let reverse = tool_reverse(&converted.blocks, &converted.names);
    Ok(json!({"tools": blocks, "reverse": reverse}))
}

fn tool_reverse(blocks: &[Json], names: &ToolNameMap) -> Value {
    let mut reverse = Map::new();
    for block in blocks {
        let Some(spec) = block
            .as_object()
            .and_then(|fields| fields.get("toolSpec"))
            .and_then(Json::as_object)
        else {
            continue;
        };
        let Some(name) = spec.get("name").and_then(Json::as_str) else {
            continue;
        };
        let original = names
            .get(name)
            .map_or_else(|| name.to_owned(), Clone::clone);
        reverse.insert(name.to_owned(), Value::String(original));
    }
    Value::Object(reverse)
}

fn json_value(value: &Json) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn tool_failure(error: &BedrockToolError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BedrockToolError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
