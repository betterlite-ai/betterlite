pub mod beta;
pub mod billing;
pub mod cache;
pub mod compaction;
pub mod messages;
pub mod tools;

use std::collections::BTreeSet;

use betterlite_models::decisions::Json as ParamsJson;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::file_ids::limits::FileIdsError;
use serde::Deserialize;
use thiserror::Error;

mod tests;

// The optional_params keys these feature gates read; every other key belongs to the transformation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureParams {
    pub compaction: Option<ParamsJson>,
    pub container: Option<ParamsJson>,
    pub output_config: Option<ParamsJson>,
    pub reasoning_effort: Option<ParamsJson>,
}

#[derive(Debug, Error)]
pub enum FeatureFlagsError {
    #[error("eager_input_streaming must be a boolean")]
    EagerInputStreaming,
    #[error("messages do not decode as chat messages")]
    Messages(#[source] serde_json::Error),
    #[error(transparent)]
    FileIds(#[from] FileIdsError),
}

// Every content or tool shape LiteLLM turns into a beta header or a provider-side feature.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Feature {
    CacheControl,
    CodeExecution,
    ContainerWithSkills,
    EagerInputStreaming,
    FileId,
    InputExamples,
    McpServer,
    MidConversationOutputConfig,
    MidConversationToolChange,
    Pdf,
    ProgrammaticToolCalling,
    ThinkingDisplayUpdates,
    ToolSearch,
    WebSearch,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnthropicFeatures {
    pub flags: BTreeSet<Feature>,
    // the first computer_ tool version in the list, which names the beta header LiteLLM sends
    pub computer_tool: Option<String>,
}

pub struct FeatureRequest<'request> {
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
    pub params: Option<&'request FeatureParams>,
    pub thinking: Option<&'request Json>,
    pub mcp_servers: Option<&'request [Json]>,
}

pub fn detect(request: &FeatureRequest<'_>) -> Result<AnthropicFeatures, FeatureFlagsError> {
    let mut features = AnthropicFeatures::default();
    detect_messages(request, &mut features.flags)?;
    detect_tools(request.tools, &mut features)?;
    if messages::is_mcp_server_used(request.mcp_servers) {
        features.flags.insert(Feature::McpServer);
    }
    if tools::is_container_with_skills_used(request.params) {
        features.flags.insert(Feature::ContainerWithSkills);
    }
    if messages::has_thinking_display_updates(request.thinking) {
        features.flags.insert(Feature::ThinkingDisplayUpdates);
    }
    Ok(features)
}

fn detect_messages(
    request: &FeatureRequest<'_>,
    flags: &mut BTreeSet<Feature>,
) -> Result<(), FeatureFlagsError> {
    if messages::is_cache_control_set(request.messages) {
        flags.insert(Feature::CacheControl);
    }
    if messages::is_file_id_used(request.messages)? {
        flags.insert(Feature::FileId);
    }
    if messages::has_tool_change(request.messages) {
        flags.insert(Feature::MidConversationToolChange);
    }
    if messages::has_output_config(request.messages) {
        flags.insert(Feature::MidConversationOutputConfig);
    }
    if messages::is_pdf_used(request.messages) {
        flags.insert(Feature::Pdf);
    }
    Ok(())
}

fn detect_tools(
    tools: Option<&[Json]>,
    features: &mut AnthropicFeatures,
) -> Result<(), FeatureFlagsError> {
    features.computer_tool = tools::is_computer_tool_used(tools);
    if tools::is_web_search_tool_used(tools) {
        features.flags.insert(Feature::WebSearch);
    }
    if tools::is_tool_search_used(tools) {
        features.flags.insert(Feature::ToolSearch);
    }
    if tools::uses_programmatic_calls(tools) {
        features.flags.insert(Feature::ProgrammaticToolCalling);
    }
    if tools::uses_input_examples(tools) {
        features.flags.insert(Feature::InputExamples);
    }
    if tools::is_code_execution_tool_used(tools) {
        features.flags.insert(Feature::CodeExecution);
    }
    if tools::is_eager_input_streaming_used(tools)? {
        features.flags.insert(Feature::EagerInputStreaming);
    }
    Ok(())
}
