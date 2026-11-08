use crate::fields::payload::StandardLoggingPayload;

use super::text::optional_str;

// litellm.integrations.langfuse.LangFuseLogger._get_langfuse_tags
#[must_use]
pub fn get_langfuse_tags(payload: Option<&StandardLoggingPayload>) -> Vec<String> {
    payload.map_or_else(Vec::new, |standard_logging_object| {
        standard_logging_object
            .request_tags
            .iter()
            .filter_map(|tag| optional_str(Some(tag)))
            .collect()
    })
}

// litellm.integrations.langfuse.LangFuseLogger.add_default_langfuse_tags
#[must_use]
pub fn add_default_langfuse_tags(
    mut tags: Vec<String>,
    default_tags: Option<&[String]>,
    cache_hit: Option<bool>,
    hidden_cache_key: Option<&str>,
    preset_cache_key: Option<&str>,
) -> Vec<String> {
    let Some(configured_tags) = default_tags else {
        return tags;
    };
    if configured_tags.iter().any(|tag| tag == "cache_hit") {
        let value = if cache_hit.unwrap_or(false) {
            "True"
        } else {
            "False"
        };
        tags.push(format!("cache_hit:{value}"));
    }
    if configured_tags.iter().any(|tag| tag == "cache_key") {
        let cache_key = hidden_cache_key.or(preset_cache_key).unwrap_or("None");
        tags.push(format!("cache_key:{cache_key}"));
    }
    tags
}
