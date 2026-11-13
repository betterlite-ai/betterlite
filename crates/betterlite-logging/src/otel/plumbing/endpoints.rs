use crate::otel::config::Signal;

const SIGNAL_PATHS: [&str; 3] = ["/v1/traces", "/v1/metrics", "/v1/logs"];

const fn signal_path(signal: Signal) -> &'static str {
    match signal {
        Signal::Traces => "/v1/traces",
        Signal::Metrics => "/v1/metrics",
        Signal::Logs => "/v1/logs",
    }
}

// An explicit OTLP/HTTP endpoint is used verbatim, so a base URL needs its signal path here.
#[must_use]
pub fn signal_endpoint(endpoint: Option<&str>, signal: Signal) -> Option<String> {
    let text = endpoint?;
    if text.is_empty() {
        return Some(String::new());
    }
    let base = text.trim_end_matches('/');
    let path = signal_path(signal);
    // Splunk Observability serves traces on /v2/trace/otlp and is never rewritten.
    let splunk = matches!(signal, Signal::Traces) && base.contains("/v2/trace/otlp");
    if base.ends_with(path) || splunk {
        return Some(base.to_owned());
    }
    let sibling_root = SIGNAL_PATHS
        .iter()
        .filter(|other| **other != path)
        .find_map(|other| base.strip_suffix(other));
    Some(format!("{}{path}", sibling_root.unwrap_or(base)))
}

#[path = "endpoints/tests.rs"]
mod tests;
