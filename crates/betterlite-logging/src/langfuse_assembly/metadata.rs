use std::convert::Infallible;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::langfuse_projections::masking::apply_masking_function;
use crate::langfuse_projections::steering::{as_steering_flag, as_steering_key_sequence};
use crate::langfuse_projections::tags::{add_default_langfuse_tags, get_langfuse_tags};
use crate::langfuse_projections::text::optional_str;
use crate::logging_privacy::snapshot::redact_user_api_key_info;

use super::{AssemblyInput, Masker};

pub(super) const DENIED_STEERING_KEYS: [&str; 4] =
    ["headers", "endpoint", "caching_groups", "previous_models"];

// the metadata fields _log_langfuse_v2 pops out before it builds a trace, and the flags it derives from them
pub struct Steering {
    pub session_id: Option<Json>,
    pub trace_name: Option<Json>,
    pub trace_id: Option<Json>,
    pub existing_trace_id: Option<Json>,
    pub update_trace_keys: Vec<String>,
    pub debug: bool,
    pub mask: MaskFlags,
}

// the mask_input and mask_output metadata flags
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaskFlags {
    pub input: bool,
    pub output: bool,
}

pub struct Cleaned {
    pub fields: IndexMap<String, Json>,
    pub tags: Vec<String>,
    pub steering: Steering,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Masked {
    pub input: Option<Json>,
    pub output: Option<Json>,
}

// litellm.integrations.langfuse.langfuse.LangFuseLogger._log_langfuse_v2 metadata cleaning
#[must_use]
pub fn clean(input: &AssemblyInput<'_>) -> Cleaned {
    let mut request_tags = get_langfuse_tags(input.trace.payload);
    let prompt_management = input
        .trace
        .payload
        .and_then(|payload| payload.metadata.prompt_management_metadata.as_ref());
    let mut cleaned_fields = clean_fields(
        input.trace.metadata,
        prompt_management,
        input.settings.default_tags,
        &mut request_tags,
    );
    let hidden_cache_key = hidden_cache_key(input.trace.metadata);
    let tags = add_default_langfuse_tags(
        request_tags,
        input.settings.default_tags,
        input.trace.cache_hit,
        hidden_cache_key.as_deref(),
        input.trace.preset_cache_key,
    );
    let steering = pop_steering(&mut cleaned_fields, input.settings.update_trace_keys);
    Cleaned {
        fields: redacted(cleaned_fields, input.settings.redact_user_api_key_info),
        tags,
        steering,
    }
}

fn clean_fields(
    metadata: &IndexMap<String, Json>,
    prompt_management: Option<&Json>,
    default_tags: &[String],
    tags: &mut Vec<String>,
) -> IndexMap<String, Json> {
    let mut clean = IndexMap::new();
    if let Some(value) = prompt_management
        && !matches!(value, Json::Null)
    {
        clean.insert("prompt_management_metadata".to_owned(), value.clone());
    }
    for (key, value) in metadata {
        if default_tags.contains(key) {
            tags.push(format!("{key}:{}", tag_text(value)));
        }
        if DENIED_STEERING_KEYS.contains(&key.as_str()) {
            continue;
        }
        clean.insert(key.clone(), value.clone());
    }
    clean
}

// the f-string in the tag uses Python's str(), which betterlite writes as JSON text
fn tag_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

// _cache_key falls back to litellm.cache's preset key, which the caller injects
fn hidden_cache_key(metadata: &IndexMap<String, Json>) -> Option<String> {
    let Some(Json::Object(hidden)) = metadata.get("hidden_params") else {
        return None;
    };
    optional_str(hidden.get("cache_key"))
}

// a Rust masking function cannot fail, so the projection's warning path never fires
#[must_use]
pub fn mask(masker: Option<Masker<'_>>, input: Option<&Json>, output: Option<&Json>) -> Masked {
    Masked {
        input: masked_value(masker, input),
        output: masked_value(masker, output),
    }
}

// without a configured function the value travels unmasked, as the source's `else input` branch does
fn masked_value(masker: Option<Masker<'_>>, value: Option<&Json>) -> Option<Json> {
    let Some(mask_function) = masker else {
        return value.cloned();
    };
    let unmasked = value?;
    let adapter = |node: &Json| -> Result<Json, Infallible> { Ok(mask_function(node)) };
    Some(apply_masking_function(unmasked, &adapter))
}

fn pop_steering(fields: &mut IndexMap<String, Json>, update_keys_enabled: bool) -> Steering {
    let requested = as_steering_key_sequence(fields.shift_remove("update_trace_keys").as_ref());
    // the callable itself cannot ride in JSON, so only its Python-side metadata key is dropped
    let _dropped = fields.shift_remove("langfuse_masking_function");
    Steering {
        session_id: pop_present(fields, "session_id"),
        trace_name: pop_present(fields, "trace_name"),
        trace_id: pop_present(fields, "trace_id"),
        existing_trace_id: pop_present(fields, "existing_trace_id"),
        update_trace_keys: if update_keys_enabled {
            requested
        } else {
            Vec::new()
        },
        debug: pop_present(fields, "debug_langfuse").is_some_and(|value| debug_requested(&value)),
        mask: MaskFlags {
            input: pop_present(fields, "mask_input")
                .as_ref()
                .is_some_and(as_steering_flag),
            output: pop_present(fields, "mask_output")
                .as_ref()
                .is_some_and(as_steering_flag),
        },
    }
}

fn pop_present(fields: &mut IndexMap<String, Json>, name: &str) -> Option<Json> {
    fields
        .shift_remove(name)
        .filter(|value| !matches!(value, Json::Null))
}

fn debug_requested(value: &Json) -> bool {
    match value {
        Json::Bool(true) => true,
        Json::Text(text) => text.to_lowercase() == "true",
        Json::Null | Json::Bool(false) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    }
}

fn redacted(fields: IndexMap<String, Json>, enabled: bool) -> IndexMap<String, Json> {
    match redact_user_api_key_info(&Json::Object(fields), enabled) {
        Json::Object(kept) => kept,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    }
}
