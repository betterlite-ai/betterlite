use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::envelope::facade::ResponseCache;
use betterlite_cache::handler::kwargs::key_request;
use betterlite_logging::latency_alerts::presentation::float_text;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::identifiers::Attribution;
use super::{Context, SpendCache, SpendPayloadError};

pub(crate) const CACHE_OFF: &str = "Cache OFF";
pub(crate) const PRESET_CACHE_KEY: &str = "preset_cache_key";
const CACHE_HIT_SUFFIX: &str = "_cache_hit";
const MISSING_ID: &str = "None";

// The request id: the base id, or Python's None text, with the cache-hit suffix that keeps the row unique.
pub(crate) fn request_id(context: &Context<'_>, attribution: &Attribution) -> String {
    let base = attribution
        .base_id
        .clone()
        .unwrap_or_else(|| MISSING_ID.to_owned());
    match context.cache_hit {
        Some(true) => {
            let epoch = float_text(context.request.clock.now_epoch_seconds());
            format!("{base}{CACHE_HIT_SUFFIX}{epoch}")
        }
        Some(false) | None => base,
    }
}

// litellm.cache: no cache reads as Cache OFF, a non-null preset key wins as given, else the cache derives one.
pub(crate) fn key(
    context: &Context<'_>,
    cache: Option<&dyn SpendCache>,
) -> Result<String, SpendPayloadError> {
    let Some(configured) = cache else {
        return Ok(CACHE_OFF.to_owned());
    };
    match context.litellm_params.get(PRESET_CACHE_KEY) {
        None | Some(Json::Null) => configured.cache_key(context.kwargs),
        Some(Json::Text(preset)) => Ok(preset.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(SpendPayloadError::FieldType {
                field: PRESET_CACHE_KEY,
            })
        }
    }
}

// The configured cache derives the key from the same kwargs get_cache_key(**kwargs) receives.
impl<L, C> SpendCache for ResponseCache<L, C>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    fn cache_key(&self, kwargs: &JsonObject) -> Result<String, SpendPayloadError> {
        let mut request = key_request(&key_object(kwargs))?;
        Ok(get_cache_key(
            &mut request,
            &self.key_settings,
            &self.hooks,
        )?)
    }
}

fn key_object(kwargs: &JsonObject) -> KeyObject {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), key_value(value)))
        .collect()
}

fn key_value(value: &Json) -> KeyValue {
    match value {
        Json::Null => KeyValue::Null,
        Json::Bool(flag) => KeyValue::Bool(*flag),
        Json::Number(number) => KeyValue::Number(number.clone()),
        Json::Text(text) => KeyValue::String(text.clone()),
        Json::List(items) => KeyValue::Array(items.iter().map(key_value).collect()),
        Json::Object(fields) => KeyValue::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), key_value(item)))
                .collect(),
        ),
    }
}
