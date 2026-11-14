use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;

use super::metadata::{Masked, Steering};
use super::trace_ids::CallTrace;
use super::{AssemblyInput, REDACTED_BY_LITELLM};

pub struct TraceParams {
    pub params: IndexMap<String, Json>,
}

pub struct TracePlanRequest<'request> {
    pub input: &'request AssemblyInput<'request>,
    pub steering: &'request Steering,
    pub call: &'request CallTrace,
    pub masked: &'request Masked,
    pub environment: &'request dyn EnvironmentLookup,
}

// litellm.integrations.langfuse.langfuse.LangFuseLogger._log_langfuse_v2 trace parameter assembly
#[must_use]
pub fn build(
    request: &TracePlanRequest<'_>,
    fields: &mut IndexMap<String, Json>,
    tags: &mut Vec<String>,
) -> TraceParams {
    let mut params = match &request.call.existing {
        Some(existing) => existing_params(request, existing, fields),
        None => new_params(request, fields),
    };
    merge_debug_metadata(
        &mut params,
        request.input.trace.metadata,
        request.steering.debug,
    );
    add_proxy_base_tag(request, tags);
    if request.call.existing.is_none() {
        let values = tags.iter().cloned().map(Json::Text).collect();
        params.insert("tags".to_owned(), Json::List(values));
    }
    TraceParams { params }
}

fn new_params(
    request: &TracePlanRequest<'_>,
    fields: &mut IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    insert_optional(
        &mut params,
        "id",
        request
            .call
            .call_trace_id
            .as_ref()
            .map(|id| Json::Text(id.clone())),
    );
    let name = request
        .steering
        .trace_name
        .clone()
        .unwrap_or_else(|| Json::Text(format!("litellm-{}", request.input.trace.call_type)));
    params.insert("name".to_owned(), name);
    insert_optional(
        &mut params,
        "session_id",
        request.steering.session_id.clone(),
    );
    params.insert(
        "input".to_owned(),
        masked_io(request.masked.input.as_ref(), request.steering.mask.input),
    );
    insert_optional(&mut params, "version", trace_version(fields));
    insert_optional(
        &mut params,
        "user_id",
        end_user_id(request.input.trace.payload),
    );
    for key in trace_keys(fields) {
        let value = fields.shift_remove(&key).unwrap_or(Json::Null);
        params.insert(key.replace("trace_", ""), value);
    }
    params.insert(
        "output".to_owned(),
        masked_io(request.masked.output.as_ref(), request.steering.mask.output),
    );
    params
}

fn existing_params(
    request: &TracePlanRequest<'_>,
    existing: &str,
    fields: &mut IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert("id".to_owned(), Json::Text(existing.to_owned()));
    for metadata_key in &request.steering.update_trace_keys {
        let param_key = metadata_key.replace("trace_", "");
        if params.contains_key(&param_key) {
            continue;
        }
        if let Some(value) = pop_present(fields, metadata_key) {
            params.insert(param_key, value);
        }
    }
    fields.retain(|key, _value| !key.starts_with("trace_"));
    if request
        .steering
        .update_trace_keys
        .iter()
        .any(|key| key == "input")
    {
        params.insert(
            "input".to_owned(),
            masked_io(request.masked.input.as_ref(), request.steering.mask.input),
        );
    }
    if request
        .steering
        .update_trace_keys
        .iter()
        .any(|key| key == "output")
    {
        params.insert(
            "output".to_owned(),
            masked_io(request.masked.output.as_ref(), request.steering.mask.output),
        );
    }
    params
}

fn trace_keys(fields: &IndexMap<String, Json>) -> Vec<String> {
    fields
        .keys()
        .filter(|key| key.starts_with("trace_"))
        .cloned()
        .collect()
}

fn trace_version(fields: &mut IndexMap<String, Json>) -> Option<Json> {
    fields
        .shift_remove("trace_version")
        .map_or_else(|| fields.get("version").cloned(), Some)
}

fn end_user_id(payload: Option<&StandardLoggingPayload>) -> Option<Json> {
    payload
        .and_then(|logged| logged.metadata.user_api_key_end_user_id.clone())
        .map(Json::Text)
}

fn masked_io(value: Option<&Json>, redacted: bool) -> Json {
    if redacted {
        return Json::Text(REDACTED_BY_LITELLM.to_owned());
    }
    value.cloned().unwrap_or(Json::Null)
}

fn insert_optional(params: &mut IndexMap<String, Json>, name: &str, candidate: Option<Json>) {
    if let Some(value) = candidate {
        params.insert(name.to_owned(), value);
    }
}

fn pop_present(fields: &mut IndexMap<String, Json>, name: &str) -> Option<Json> {
    fields
        .shift_remove(name)
        .filter(|value| !matches!(value, Json::Null))
}

fn merge_debug_metadata(
    params: &mut IndexMap<String, Json>,
    metadata: &IndexMap<String, Json>,
    debug: bool,
) {
    if !debug {
        return;
    }
    let mut debug_fields = IndexMap::new();
    for (key, value) in metadata {
        if matches!(value, Json::Text(_) | Json::Number(_) | Json::Bool(_)) {
            debug_fields.insert(key.clone(), value.clone());
        }
    }
    let mut merged = match params.shift_remove("metadata") {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    };
    merged.insert(
        "metadata_passed_to_litellm".to_owned(),
        Json::Object(debug_fields),
    );
    params.insert("metadata".to_owned(), Json::Object(merged));
}

fn add_proxy_base_tag(request: &TracePlanRequest<'_>, tags: &mut Vec<String>) {
    if !request
        .input
        .settings
        .default_tags
        .iter()
        .any(|tag| tag == "proxy_base_url")
    {
        return;
    }
    let configured = request
        .environment
        .lookup("PROXY_BASE_URL")
        .filter(|value| !value.is_empty());
    if let Some(base) = configured {
        tags.push(format!("proxy_base_url:{base}"));
    }
}
