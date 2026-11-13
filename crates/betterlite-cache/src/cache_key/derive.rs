use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;
use sha2::{Digest as _, Sha256};

use super::error::CacheKeyError;
use super::material;
use super::request::{CacheKeyRequest, RequestCacheControl};
use super::settings::CacheKeySettings;
use super::value::{KeyValue, field};
use crate::types::cache_type::CacheMode;

// LiteLLM's get_cache_key: a preset key bypasses derivation, otherwise the request material is hashed and namespaced.
pub fn get_cache_key<L, C>(
    request: &mut CacheKeyRequest,
    settings: &CacheKeySettings,
    hooks: &VerboseHooks<L, C>,
) -> Result<String, CacheKeyError>
where
    L: LoggerSink,
    C: ConsoleSink,
{
    if let Some(preset) = request
        .litellm_params
        .as_ref()
        .and_then(|params| params.preset_cache_key.clone())
    {
        let message = format!("returning preset cache key: {preset}");
        // LiteLLM only debug-logs the derivation steps, so logger_only keeps set_verbose from adding console lines.
        let _logged =
            hooks.utils_print_verbose(Payload::String(message), "DEBUG", &Payload::Bool(true));
        return Ok(preset);
    }
    let semantic = settings
        .is_semantic_cache()
        .then_some(settings.semantic_cache_scope);
    let key_material = material::build(request, settings.include_provider_params, semantic)?;
    let hashed = hashed_key(&key_material, hooks);
    let key = add_namespace(request, settings, hooks, hashed);
    let length = key_material.len();
    let message = format!("created cache key: {key} (source material length: {length})");
    let _logged =
        hooks.utils_print_verbose(Payload::String(message), "DEBUG", &Payload::Bool(true));
    if let Some(params) = request.litellm_params.as_mut() {
        params.preset_cache_key = Some(key.clone());
    }
    Ok(key)
}

#[must_use]
pub fn should_use_cache(
    settings: &CacheKeySettings,
    control: Option<&RequestCacheControl>,
) -> bool {
    match settings.mode {
        CacheMode::DefaultOn => true,
        CacheMode::DefaultOff => control.is_some_and(|request_control| request_control.use_cache),
    }
}

fn hashed_key<L, C>(key_material: &str, hooks: &VerboseHooks<L, C>) -> String
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let hashed = hex::encode(Sha256::digest(key_material.as_bytes()));
    let message = format!("hashed cache key (SHA-256): {hashed}");
    let _logged =
        hooks.utils_print_verbose(Payload::String(message), "DEBUG", &Payload::Bool(true));
    hashed
}

// A request namespace wins over metadata.redis_namespace, which wins over the cache's own namespace.
fn add_namespace<L, C>(
    request: &CacheKeyRequest,
    settings: &CacheKeySettings,
    hooks: &VerboseHooks<L, C>,
    hashed: String,
) -> String
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let request_namespace = request
        .cache_control
        .as_ref()
        .and_then(|control| control.namespace.clone());
    let metadata_namespace =
        field(request.metadata.as_ref(), "redis_namespace").map(KeyValue::render);
    let namespace = request_namespace
        .or(metadata_namespace)
        .or_else(|| settings.namespace.clone());
    let key = match namespace.as_deref() {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{hashed}"),
        Some(_) | None => hashed,
    };
    let message = format!("final hashed key: {key}");
    let _logged =
        hooks.utils_print_verbose(Payload::String(message), "DEBUG", &Payload::Bool(true));
    key
}
