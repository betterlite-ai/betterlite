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
    // LiteLLM leaves Splunk's newer trace path and Langtrace's trace path exactly as configured.
    let splunk = base.contains("/v2/trace/otlp");
    let langtrace = callback_name == Some("langtrace") && base.ends_with(LANGTRACE_TRACE_PATH);
    if matches!(signal, Signal::Traces) && (splunk || langtrace) {
        return Some(base.to_owned());
    }
    let suffix = signal_suffix(signal);
    let target = format!("{V1_PREFIX}{suffix}");
    if base.ends_with(target.as_str()) {
        return Some(base.to_owned());
    }
    // A sibling path is only swapped when it is the whole final segment, so /v1/logs/extra is appended to.
    let siblings = match signal {
        Signal::Traces => ["/metrics", "/logs"],
        Signal::Metrics => ["/traces", "/logs"],
        Signal::Logs => ["/traces", "/metrics"],
    };
    for other in siblings {
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
