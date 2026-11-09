use crate::otel::config::Signal;

pub const LANGTRACE_TRACE_PATH: &str = "/api/trace";

const V1_PREFIX: &str = "/v1";

const fn signal_suffix(signal: Signal) -> &'static str {
    match signal {
        Signal::Traces => "/traces",
        Signal::Metrics => "/metrics",
        Signal::Logs => "/logs",
    }
}

// LiteLLM leaves Splunk's newer trace path and Langtrace's trace path exactly as configured.
fn keeps_its_path(base: &str, signal: Signal, callback_name: Option<&str>) -> bool {
    if !matches!(signal, Signal::Traces) {
        return false;
    }
    base.contains("/v2/trace/otlp")
        || (callback_name == Some("langtrace") && base.ends_with(LANGTRACE_TRACE_PATH))
}

// A sibling path is only swapped when it is the whole final segment, so /v1/logs/extra is appended to.
const fn sibling_suffixes(signal: Signal) -> [&'static str; 2] {
    match signal {
        Signal::Traces => ["/metrics", "/logs"],
        Signal::Metrics => ["/traces", "/logs"],
        Signal::Logs => ["/traces", "/metrics"],
    }
}

#[must_use]
pub fn normalize_otel_endpoint(
    endpoint: Option<&str>,
    signal: Signal,
    callback_name: Option<&str>,
) -> Option<String> {
    let text = endpoint?;
    if text.is_empty() {
        return Some(String::new());
    }
    let base = text.trim_end_matches('/');
    if keeps_its_path(base, signal, callback_name) {
        return Some(base.to_owned());
    }
    let suffix = signal_suffix(signal);
    let target = format!("{V1_PREFIX}{suffix}");
    if base.ends_with(target.as_str()) {
        return Some(base.to_owned());
    }
    for other in sibling_suffixes(signal) {
        let other_path = format!("{V1_PREFIX}{other}");
        if let Some(root) = base.strip_suffix(other_path.as_str()) {
            return Some(format!("{root}{target}"));
        }
    }
    if base.ends_with(V1_PREFIX) {
        return Some(format!("{base}{suffix}"));
    }
    Some(format!("{base}{target}"))
}

#[path = "endpoint/tests.rs"]
mod tests;
