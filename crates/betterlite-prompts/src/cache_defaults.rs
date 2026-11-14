pub mod attribution;
pub mod chat;
pub mod messages;
pub mod selection;

mod tests;

use thiserror::Error;

use crate::arguments::json::Json;
use crate::cache_accounting::dialect::ModelFacts;
use crate::cache_accounting::points::CacheControlInjectionPoint;

pub const INJECTION_POINTS: &str = "cache_control_injection_points";

// The globals a request sees: the config flag and the already-resolved ttl policy.
#[derive(Clone, Copy)]
pub struct CachePolicy<'input> {
    pub global_enabled: bool,
    pub ttl: Option<&'input str>,
}

// What a caller must resolve from its model catalog beyond the dialect facts.
pub trait CacheFacts: ModelFacts {
    fn supports_prompt_caching(&self, model: &str, provider: &str) -> bool;
}

#[derive(Debug, Error)]
pub enum DefaultsError {
    #[error("cache_control_injection_points is not a list of injection points")]
    InjectionPoints {
        #[source]
        source: serde_json::Error,
    },
    #[error("an injection point could not be written back into the request")]
    PointEncoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("enable_prompt_caching is not a boolean")]
    EnablePromptCaching,
}

pub(crate) fn encode_points(points: &[CacheControlInjectionPoint]) -> Result<Json, DefaultsError> {
    let value =
        serde_json::to_value(points).map_err(|source| DefaultsError::PointEncoding { source })?;
    serde_json::from_value(value).map_err(|source| DefaultsError::PointEncoding { source })
}
