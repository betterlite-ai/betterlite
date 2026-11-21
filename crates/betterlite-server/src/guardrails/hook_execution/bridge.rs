use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::execute::HookExecution;
use crate::guardrail_message_scope::exception::{
    FastapiInstall, GuardrailCallback as ErrorContext, HttpException, RaisedError,
    add_guardrail_context,
};
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::guardrails::callback_caps::traits::{GuardrailCallback, ProxyCallback};

const METADATA_BUCKETS: [&str; 2] = ["metadata", "litellm_metadata"];

// the recorders write only into these two buckets, so only they cross into guardrail values
pub fn metadata_buckets(
    data: &JsonMap,
) -> Result<BTreeMap<String, GuardrailValue>, serde_json::Error> {
    let buckets: JsonMap = METADATA_BUCKETS
        .iter()
        .filter_map(|key| {
            data.get(*key)
                .map(|value| ((*key).to_owned(), value.clone()))
        })
        .collect();
    // OpaqueParams reads a number as a number; the derived GuardrailValue reads an arbitrary_precision number as a map
    serde_json::from_value::<OpaqueParams>(JsonValue::Object(buckets)).map(|params| params.0)
}

pub fn write_buckets(
    data: &mut JsonMap,
    buckets: &BTreeMap<String, GuardrailValue>,
) -> Result<(), serde_json::Error> {
    for (key, value) in buckets {
        data.insert(key.clone(), serde_json::to_value(value)?);
    }
    Ok(())
}

// guardrails that run together each get a copy; what one wrote into a metadata bucket reaches the live request
pub fn merge_metadata(live: &mut JsonMap, shared: &JsonMap, run: &JsonMap) {
    for key in METADATA_BUCKETS {
        let Some(JsonValue::Object(changed)) = run.get(key) else {
            continue;
        };
        let before = shared.get(key).and_then(JsonValue::as_object);
        let bucket = live.entry(key.to_owned()).or_insert(JsonValue::Null);
        if !bucket.is_object() {
            *bucket = JsonValue::Object(JsonMap::new());
        }
        let Some(fields) = bucket.as_object_mut() else {
            continue;
        };
        for (field, value) in changed {
            if before.and_then(|existing| existing.get(field)) != Some(value) {
                merge_field(fields, field, value);
            }
        }
    }
}

// a list keeps the entries of every guardrail, so two markers or two records never overwrite each other
fn merge_field(fields: &mut JsonMap, field: &str, value: &JsonValue) {
    if let (Some(JsonValue::Array(existing)), JsonValue::Array(added)) =
        (fields.get_mut(field), value)
    {
        for item in added {
            if !existing.contains(item) {
                existing.push(item.clone());
            }
        }
        return;
    }
    fields.insert(field.to_owned(), value.clone());
}

// LiteLLM's _record_raising_guardrail; a reroute is not a raise
pub fn record_raising_guardrail(
    execution: &HookExecution,
    data: &mut JsonMap,
    callback: &dyn ProxyCallback,
    error: &ProxyHookError,
) {
    if matches!(error, ProxyHookError::SensitiveDataRoute(_)) {
        return;
    }
    let Some(name) = callback
        .guardrail()
        .and_then(GuardrailCallback::guardrail_name)
    else {
        return;
    };
    let recorded = metadata_buckets(data).and_then(|mut buckets| {
        execution.recorder.record(&mut buckets, Some(name));
        write_buckets(data, &buckets)
    });
    if let Err(failure) = recorded {
        tracing::warn!("the raising guardrail {name} was not recorded: {failure}");
    }
}

// B3663's enrich_http_exception_with_guardrail_context, for an HTTP error whose detail is an object
pub fn enrich_error(
    execution: &HookExecution,
    error: &mut ProxyHookError,
    callback: &dyn ProxyCallback,
) {
    let ProxyHookError::Http { status, detail } = error else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<OpaqueParams>(detail.as_str()) else {
        return;
    };
    let event_hook = execution.bases.base(callback).and_then(|base| {
        let mode = base.settings.event_hook.as_ref()?;
        serde_json::to_value(mode)
            .and_then(serde_json::from_value)
            .ok()
    });
    let context = ErrorContext {
        guardrail_name: callback
            .guardrail()
            .and_then(GuardrailCallback::guardrail_name)
            .map(str::to_owned),
        event_hook,
    };
    let mut raised = RaisedError::Http(HttpException {
        status_code: *status,
        detail: GuardrailValue::Object(parsed.0),
    });
    add_guardrail_context(&mut raised, &context, FastapiInstall::Installed);
    if let RaisedError::Http(enriched) = raised
        && let Ok(text) = serde_json::to_string(&enriched.detail)
    {
        *detail = text;
    }
}
