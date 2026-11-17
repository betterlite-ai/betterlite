use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::converse_content::ToolBlock;

// The optional_params members this conversion reads; the request tasks own every other member.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct OptionalParams {
    pub json_mode: Option<bool>,
    pub tools: Option<Vec<ToolBlock>>,
}

pub struct ReplyRequest<'request> {
    pub model: &'request str,
    pub messages: &'request Json,
    pub data: RawBody,
    pub json_mode: Option<bool>,
    pub optional: &'request OptionalParams,
    pub tool_name_mappings: &'request IndexMap<String, String>,
}

pub struct RawReply<'reply> {
    pub headers: &'reply [(String, String)],
    pub body: &'reply [u8],
}

// The explicit json_mode argument wins whenever it is set, even as false.
impl ReplyRequest<'_> {
    #[must_use]
    pub fn resolved_json_mode(&self) -> bool {
        self.json_mode.or(self.optional.json_mode).unwrap_or(false)
    }

    #[must_use]
    pub fn tools(&self) -> Option<&[ToolBlock]> {
        self.optional.tools.as_deref()
    }
}
