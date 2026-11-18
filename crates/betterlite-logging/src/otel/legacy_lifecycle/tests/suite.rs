#![cfg(test)]

mod dynamic_config;
mod dynamic_headers;
mod guardrail_spans;
mod legacy_lifecycle;
mod otel {
    pub mod legacy_lifecycle {
        mod logger;
    }
}
mod proxy_span;
mod support;
mod tracer_selection;
