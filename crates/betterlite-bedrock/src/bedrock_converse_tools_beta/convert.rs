use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptsJson;
use betterlite_prompts::bedrock_tools::capabilities::BedrockToolCapabilities;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::bedrock_tools::tools::bedrock_tools;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::error::ToolsAndBetaError;
use crate::bedrock_converse_tools_beta::json::convert_all;
use crate::converse_tools::computer_use::{
    is_computer_use_tool_used, separate_computer_use_tools, transform_computer_use_tools,
};

const COMPUTER_2025_11_24: &str = "computer-use-2025-11-24";
const COMPUTER_2025_01_24: &str = "computer-use-2025-01-24";
const COMPUTER_2024_10_22: &str = "computer-use-2024-10-22";

const LATEST_MARKERS: [&str; 16] = [
    "opus-4.7",
    "opus_4.7",
    "opus-4-7",
    "opus_4_7",
    "opus-4.6",
    "opus_4.6",
    "opus-4-6",
    "opus_4_6",
    "sonnet-4.6",
    "sonnet_4.6",
    "sonnet-4-6",
    "sonnet_4_6",
    "opus-4.5",
    "opus_4.5",
    "opus-4-5",
    "opus_4_5",
];

const MID_MARKERS: [&str; 20] = [
    "sonnet-4.5",
    "sonnet_4.5",
    "sonnet-4-5",
    "sonnet_4_5",
    "haiku-4.5",
    "haiku_4.5",
    "haiku-4-5",
    "haiku_4_5",
    "opus-4.1",
    "opus_4.1",
    "opus-4-1",
    "opus_4_1",
    "sonnet-4",
    "sonnet_4",
    "opus-4",
    "opus_4",
    "sonnet-3.7",
    "sonnet_3.7",
    "sonnet-3-7",
    "sonnet_3_7",
];

pub struct ConvertedTools {
    pub blocks: Vec<Json>,
    pub names: ToolNameMap,
    pub computer_beta: Option<&'static str>,
}

pub fn convert_tools(
    filtered: &[Json],
    prompts: &[PromptsJson],
    model: &str,
    capabilities: BedrockToolCapabilities,
    params: &mut IndexMap<String, Json>,
) -> Result<ConvertedTools, ToolsAndBetaError> {
    if !is_computer_use_tool_used(Some(filtered))? {
        return regular_blocks(prompts, capabilities);
    }
    let (computer, regular) = separate_computer_use_tools(filtered)?;
    let regular_prompts: Vec<PromptsJson> = convert_all(&regular)?;
    let converted = regular_blocks(&regular_prompts, capabilities)?;
    // Hosted computer tools travel in additionalModelRequestFields, not in toolConfig.
    params.insert(
        "tools".to_owned(),
        Json::List(transform_computer_use_tools(&computer)?),
    );
    Ok(ConvertedTools {
        blocks: converted.blocks,
        names: converted.names,
        computer_beta: Some(computer_use_beta(model)),
    })
}

fn regular_blocks(
    prompts: &[PromptsJson],
    capabilities: BedrockToolCapabilities,
) -> Result<ConvertedTools, ToolsAndBetaError> {
    let converted = bedrock_tools(prompts, capabilities)?;
    let blocks: Vec<Json> = convert_all(&converted.blocks)?;
    Ok(ConvertedTools {
        blocks,
        names: converted.names,
        computer_beta: None,
    })
}

#[must_use]
pub fn computer_use_beta(model: &str) -> &'static str {
    let lowered = model.to_lowercase();
    if LATEST_MARKERS.iter().any(|marker| lowered.contains(marker)) {
        return COMPUTER_2025_11_24;
    }
    if MID_MARKERS.iter().any(|marker| lowered.contains(marker)) {
        return COMPUTER_2025_01_24;
    }
    COMPUTER_2024_10_22
}
