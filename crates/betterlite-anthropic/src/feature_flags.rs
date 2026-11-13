pub mod beta;
pub mod billing;
pub mod cache;
pub mod compaction;
pub mod messages;
pub mod tools;

use std::collections::BTreeSet;

use betterlite_models::decisions::Json as ParamsJson;
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

impl FeatureFlagsError {
    // LiteLLM raises UnsupportedParamsError, a 400, only for a malformed eager_input_streaming
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::EagerInputStreaming => Some(400),
            Self::Messages(_) | Self::FileIds(_) => None,
        }
    }
}

// The content and tool shapes that turn into a beta header in get_anthropic_beta_list.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Feature {
    FileId,
    McpServer,
    MidConversationOutputConfig,
    ThinkingDisplayUpdates,
    MidConversationToolChange,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnthropicFeatures {
    pub flags: BTreeSet<Feature>,
    // the first computer_ tool version in the list, which names the beta header LiteLLM sends
    pub computer_tool: Option<String>,
}
