use serde::Deserialize;

use crate::arguments::json::{Json, Object};
use crate::bedrock_tool_results::invoke::CachePoints;
use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};

const TYPE: &str = "type";
const DEFAULT: &str = "default";
const TTL: &str = "ttl";
const ONE_HOUR: &str = "1h";
const FIVE_MINUTES: &str = "5m";

// The model-cost-map facts LiteLLM reads, resolved by the caller that routed the request
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CachePointCapability {
    #[default]
    Unsupported,
    ShortTtl,
    ExtendedTtl,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PromptCapabilities {
    #[serde(default)]
    pub cache_points: CachePointCapability,
    #[serde(default)]
    pub supports_strict_tools: bool,
}

impl From<CachePointCapability> for CachePointSupport {
    fn from(capability: CachePointCapability) -> Self {
        match capability {
            CachePointCapability::Unsupported => Self::Unsupported,
            CachePointCapability::ShortTtl => Self::ShortTtl,
            CachePointCapability::ExtendedTtl => Self::ExtendedTtl,
        }
    }
}

impl From<PromptCapabilities> for BedrockToolCapabilities {
    fn from(capabilities: PromptCapabilities) -> Self {
        Self {
            cache_points: CachePointSupport::from(capabilities.cache_points),
            supports_strict_tools: capabilities.supports_strict_tools,
        }
    }
}

pub struct MessageCachePoints {
    pub support: CachePointSupport,
}

impl CachePoints for MessageCachePoints {
    fn cache_point_block(&self, cache_control: &Json, model: Option<&str>) -> Option<Object> {
        let ttl_allowed = match self.support {
            CachePointSupport::Unsupported => return None,
            CachePointSupport::ShortTtl => false,
            CachePointSupport::ExtendedTtl => true,
        };
        let mut point = Object::from_iter([(TYPE.to_owned(), Json::Text(DEFAULT.to_owned()))]);
        // LiteLLM only reads a TTL when the request names a model and the model takes the extended one
        if ttl_allowed
            && model.is_some()
            && let Json::Object(control) = cache_control
            && let Some(ttl) = control.get(TTL).and_then(Json::as_str)
            && matches!(ttl, ONE_HOUR | FIVE_MINUTES)
        {
            point.insert(TTL.to_owned(), Json::Text(ttl.to_owned()));
        }
        Some(point)
    }
}
