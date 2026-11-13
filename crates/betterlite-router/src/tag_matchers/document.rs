use serde::{Deserialize, Serialize};

use super::candidates::DeploymentTags;
use super::error::TagMatcherError;

pub const MAX_TAG_MATCH_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagMatchRequest {
    #[serde(default)]
    pub deployments: Vec<DeploymentTags>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub inherited_tags: Option<Vec<String>>,
    #[serde(default)]
    pub header_strings: Vec<String>,
    #[serde(default = "default_match_any")]
    pub match_any: bool,
    #[serde(default)]
    pub routing_prefix: String,
}

const fn default_match_any() -> bool {
    true
}

pub fn parse_tag_match(bytes: &[u8]) -> Result<TagMatchRequest, TagMatcherError> {
    if bytes.len() > MAX_TAG_MATCH_BYTES {
        return Err(TagMatcherError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_TAG_MATCH_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| TagMatcherError::Invalid { source })
}
