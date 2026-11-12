use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::InputRequest;
use betterlite_core::api::responses_input_messages::input_messages;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
pub(crate) struct CacheEntry {
    key: String,
    value: CacheValue,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    request: InputRequest,
    #[serde(default)]
    replay_reasoning: bool,
    #[serde(default)]
    cache: Vec<CacheEntry>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cache = tool_cache(&input.cache).or_value_error()?;
        let messages = input_messages(
            &input.request,
            input.replay_reasoning,
            ApiLimits::default(),
            &cache,
        )
        .or_value_error()?;
        Ok(json!(messages))
    }
}

fn tool_cache(entries: &[CacheEntry]) -> Result<DualLocalCache, RedactionError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let cache = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        },
    );
    for entry in entries {
        cache.set(&entry.key, entry.value.clone(), Ttl::Absent);
    }
    Ok(cache)
}
