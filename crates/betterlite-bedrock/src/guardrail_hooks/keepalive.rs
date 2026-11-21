use std::time::Duration;

// LiteLLM's keepalive_ping_has_fired: no usable interval means pings are off, so nothing flushed the response headers.
#[must_use]
pub fn keepalive_ping_has_fired(elapsed: Duration, interval_seconds: Option<f64>) -> bool {
    interval_seconds
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .is_some_and(|interval| elapsed.as_secs_f64() >= interval)
}
