use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

const TAG_PASSTHROUGH: &str = "_-:./";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatadogEnvironment {
    pub source: String,
    pub service: String,
    pub hostname: String,
    pub environment: String,
    pub pod_name: String,
    pub version: String,
    pub base_url: Option<String>,
}

impl DatadogEnvironment {
    pub fn from_lookup(lookup: &dyn EnvironmentLookup) -> Self {
        let source = value_or(lookup, "DD_SOURCE", "litellm");
        let service = value_or(lookup, "DD_SERVICE", "litellm-server");
        let hostname = value_or(lookup, "HOSTNAME", "");
        let environment = value_or(lookup, "DD_ENV", "unknown");
        let pod_name = value_or(lookup, "POD_NAME", "unknown");
        let version = value_or(lookup, "DD_VERSION", "unknown");
        let base_url = lookup.lookup("DD_BASE_URL");
        Self {
            source,
            service,
            hostname,
            environment,
            pod_name,
            version,
            base_url,
        }
    }
}

#[must_use]
pub fn get_datadog_tags(
    environment: &DatadogEnvironment,
    request_tags: &[String],
    team_tag: Option<&str>,
) -> Vec<String> {
    let mut tags = vec![
        format!("env:{}", environment.environment),
        format!("service:{}", environment.service),
        format!("version:{}", environment.version),
        format!("HOSTNAME:{}", environment.hostname),
        format!("POD_NAME:{}", environment.pod_name),
    ];
    tags.extend(
        request_tags
            .iter()
            .map(|tag| format!("request_tag:{}", normalize_datadog_tag_value(tag))),
    );
    if let Some(team) = team_tag {
        tags.push(format!("team:{}", normalize_datadog_tag_value(team)));
    }
    tags
}

fn value_or(lookup: &dyn EnvironmentLookup, name: &str, default: &str) -> String {
    let fallback = default.to_owned();
    lookup.lookup(name).unwrap_or(fallback)
}

#[must_use]
pub fn normalize_datadog_tag_value(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.to_lowercase().chars() {
        if character.is_alphanumeric() || TAG_PASSTHROUGH.contains(character) {
            normalized.push(character);
        } else {
            normalized.push('_');
        }
    }
    let mut collapsed = String::with_capacity(normalized.len());
    for character in normalized.chars() {
        if character == '_' && collapsed.ends_with('_') {
            continue;
        }
        collapsed.push(character);
    }
    collapsed.trim_matches('_').to_owned()
}

pub trait Span: Send + Sync {
    fn set_tag_str(&self, _key: &str, _value: &str) -> bool {
        false
    }

    fn set_tag(&self, key: &str, value: &str) -> bool {
        self.set_tag_str(key, value)
    }

    fn trace_id(&self) -> Option<String> {
        None
    }

    fn span_id(&self) -> Option<String> {
        None
    }
}

pub trait Tracer: Send + Sync {
    fn current_span(&self) -> Option<Arc<dyn Span>> {
        None
    }

    fn current_root_span(&self) -> Option<Arc<dyn Span>> {
        None
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullTracer;

impl Tracer for NullTracer {}

pub fn tracer_from_lookup(lookup: &dyn EnvironmentLookup) -> NullTracer {
    let _use_trace = enabled(lookup, "USE_DDTRACE");
    let _use_profiler = enabled(lookup, "USE_DDPROFILER");
    NullTracer
}

fn enabled(lookup: &dyn EnvironmentLookup, name: &str) -> bool {
    let configured = lookup.lookup(name);
    configured.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub fn active_span(tracer: &dyn Tracer) -> Option<Arc<dyn Span>> {
    let current = catch_unwind(AssertUnwindSafe(|| tracer.current_span()))
        .ok()
        .flatten();
    if current.is_some() {
        return current;
    }
    catch_unwind(AssertUnwindSafe(|| tracer.current_root_span()))
        .ok()
        .flatten()
}

pub fn set_active_span_tag(tracer: &dyn Tracer, key: &str, value: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let Some(span) = active_span(tracer) else {
        return false;
    };
    let set_tag_str =
        catch_unwind(AssertUnwindSafe(|| span.set_tag_str(key, value))).unwrap_or(false);
    if set_tag_str {
        return true;
    }
    catch_unwind(AssertUnwindSafe(|| span.set_tag(key, value))).unwrap_or(false)
}

pub fn active_trace_context(tracer: &dyn Tracer) -> Option<BTreeMap<String, String>> {
    let span = active_span(tracer)?;
    let trace_id = catch_unwind(AssertUnwindSafe(|| span.trace_id()))
        .ok()
        .flatten()?;
    let mut context = BTreeMap::from([(String::from("trace_id"), trace_id)]);
    if let Some(span_id) = catch_unwind(AssertUnwindSafe(|| span.span_id()))
        .ok()
        .flatten()
    {
        context.insert(String::from("span_id"), span_id);
    }
    Some(context)
}

pub fn add_trace_context_to_payload(tracer: &dyn Tracer, payload: &mut BTreeMap<String, String>) {
    if let Some(context) = active_trace_context(tracer) {
        for (key, value) in context {
            let payload_key = format!("dd.{key}");
            payload.insert(payload_key, value);
        }
    }
}

pub fn tag_call_id(tracer: &dyn Tracer, call_id: Option<String>) {
    if let Some(call_id_value) = call_id {
        let _tag_was_set = set_active_span_tag(tracer, "litellm.call_id", &call_id_value);
    }
}

pub fn tag_request(
    tracer: &dyn Tracer,
    key_alias: Option<String>,
    key_hash: Option<String>,
    requested_model: Option<String>,
) {
    if let Some(alias_value) = key_alias {
        let _tag_was_set = set_active_span_tag(tracer, "litellm.key_alias", &alias_value);
    }
    if let Some(hash_value) = key_hash {
        let _tag_was_set = set_active_span_tag(tracer, "litellm.key_hash", &hash_value);
    }
    if let Some(model_value) = requested_model {
        let _tag_was_set = set_active_span_tag(tracer, "litellm.requested_model", &model_value);
    }
}

mod tests;
