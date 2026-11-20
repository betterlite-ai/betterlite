use std::collections::BTreeMap;

use super::super::CatalogInputs;

// Every variable the legacy builders read, since they take a map and the process environment stays untouched.
const LEGACY_KEYS: &[&str] = &[
    "ARIZE_API_KEY",
    "ARIZE_ENDPOINT",
    "ARIZE_HTTP_ENDPOINT",
    "ARIZE_PROJECT_NAME",
    "ARIZE_SPACE_ID",
    "ARIZE_SPACE_KEY",
    "LANGFUSE_HOST",
    "LANGFUSE_OTEL_HOST",
    "LANGFUSE_PUBLIC_KEY",
    "LANGFUSE_SECRET_KEY",
    "LANGTRACE_API_HOST",
    "LANGTRACE_API_KEY",
    "LEVOAI_API_KEY",
    "LEVOAI_COLLECTOR_URL",
    "LEVOAI_ORG_ID",
    "LEVOAI_WORKSPACE_ID",
    "LITELLM_HTTP2",
    "LITELLM_LOGGER_NAME",
    "LITELLM_OTEL_BAGGAGE_METADATA_KEYS",
    "LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS",
    "LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS",
    "LITELLM_OTEL_INTEGRATION_ENABLE_METRICS",
    "LITELLM_USER_AGENT",
    "LOGFIRE_BASE_URL",
    "LOGFIRE_TOKEN",
    "OTEL_ENDPOINT",
    "OTEL_ENVIRONMENT_NAME",
    "OTEL_EXPORTER",
    "OTEL_EXPORTER_OTLP_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_METRICS_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
    "OTEL_HEADERS",
    "OTEL_IGNORE_CONTEXT_PROPAGATION",
    "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT",
    "OTEL_LOGS_EXPORTER",
    "OTEL_MODEL_ID",
    "OTEL_RESOURCE_ATTRIBUTES",
    "OTEL_SEMCONV_STABILITY_OPT_IN",
    "OTEL_SERVICE_NAME",
    "OTEL_TRACER_NAME",
    "PHOENIX_API_KEY",
    "PHOENIX_COLLECTOR_ENDPOINT",
    "PHOENIX_COLLECTOR_HTTP_ENDPOINT",
    "PHOENIX_COLLECTOR_PROJECT_NAME",
    "PHOENIX_PROJECT_NAME",
    "SIGNOZ_INGESTION_ENDPOINT",
    "SIGNOZ_INGESTION_KEY",
    "SSL_CERT_FILE",
    "SSL_CERTIFICATE",
    "SSL_ECDH_CURVE",
    "SSL_SECURITY_LEVEL",
    "SSL_VERIFY",
    "WANDB_API_KEY",
    "WANDB_HOST",
    "WANDB_PROJECT_ID",
];

// The legacy builders and B3848's HttpClients take a map, so the injected lookup is read once into one.
#[must_use]
pub fn snapshot(inputs: &CatalogInputs) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for name in LEGACY_KEYS {
        if let Some(value) = inputs.env.lookup(name) {
            values.insert((*name).to_owned(), value);
        }
    }
    values
}

// litellm.integrations.opentelemetry's LITELLM_LOGGER_NAME, which binds the semantic event logger
#[must_use]
pub fn logger_name(environment: &BTreeMap<String, String>) -> String {
    environment
        .get("LITELLM_LOGGER_NAME")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| "litellm".to_owned())
}
