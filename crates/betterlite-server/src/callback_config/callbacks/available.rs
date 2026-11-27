use serde::Serialize;

// One field of LiteLLM's AllCallbacks: the settings-page key, the registry name it writes, its params and its display name.
pub(crate) struct AvailableCallback {
    pub(crate) key: &'static str,
    pub(crate) litellm_callback_name: &'static str,
    pub(crate) ui_callback_name: &'static str,
    pub(crate) params: &'static [&'static str],
}

#[derive(Serialize)]
pub(crate) struct UiCallback {
    pub(crate) litellm_callback_name: &'static str,
    pub(crate) litellm_callback_params: &'static [&'static str],
    pub(crate) ui_callback_name: &'static str,
}

pub(crate) const AVAILABLE_CALLBACKS: [AvailableCallback; 17] = [
    AvailableCallback {
        key: "langfuse",
        litellm_callback_name: "langfuse",
        ui_callback_name: "Langfuse",
        params: &[
            "LANGFUSE_PUBLIC_KEY",
            "LANGFUSE_SECRET_KEY",
            "LANGFUSE_HOST",
        ],
    },
    AvailableCallback {
        key: "otel",
        litellm_callback_name: "otel",
        ui_callback_name: "OpenTelemetry",
        params: &[
            "OTEL_EXPORTER",
            "OTEL_EXPORTER_OTLP_PROTOCOL",
            "OTEL_ENDPOINT",
            "OTEL_TRACES_ENDPOINT",
            "OTEL_HEADERS",
        ],
    },
    AvailableCallback {
        key: "s3",
        litellm_callback_name: "s3",
        ui_callback_name: "s3 Bucket (AWS)",
        params: &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_REGION_NAME",
            "S3_LOG_PROMPTS_ONLY",
            "S3_PARTITION_GRANULARITY",
        ],
    },
    AvailableCallback {
        key: "azure_sentinel",
        litellm_callback_name: "azure_sentinel",
        ui_callback_name: "Azure Sentinel",
        params: &[
            "AZURE_SENTINEL_DCR_IMMUTABLE_ID",
            "AZURE_SENTINEL_ENDPOINT",
            "AZURE_SENTINEL_TENANT_ID",
            "AZURE_SENTINEL_CLIENT_ID",
            "AZURE_SENTINEL_CLIENT_SECRET",
            "AZURE_SENTINEL_STREAM_NAME",
        ],
    },
    AvailableCallback {
        key: "openmeter",
        litellm_callback_name: "openmeter",
        ui_callback_name: "OpenMeter",
        params: &["OPENMETER_API_ENDPOINT", "OPENMETER_API_KEY"],
    },
    AvailableCallback {
        key: "custom_callback_api",
        litellm_callback_name: "custom_callback_api",
        ui_callback_name: "Custom Callback API",
        params: &["GENERIC_LOGGER_ENDPOINT", "GENERIC_LOGGER_HEADERS"],
    },
    AvailableCallback {
        key: "generic_api",
        litellm_callback_name: "generic_api",
        ui_callback_name: "Custom Callback API",
        params: &["GENERIC_LOGGER_ENDPOINT", "GENERIC_LOGGER_HEADERS"],
    },
    AvailableCallback {
        key: "datadog",
        litellm_callback_name: "datadog",
        ui_callback_name: "Datadog",
        params: &["DD_API_KEY", "DD_SITE"],
    },
    AvailableCallback {
        key: "braintrust",
        litellm_callback_name: "braintrust",
        ui_callback_name: "Braintrust",
        params: &["BRAINTRUST_API_KEY", "BRAINTRUST_API_BASE"],
    },
    AvailableCallback {
        key: "langsmith",
        litellm_callback_name: "langsmith",
        ui_callback_name: "Langsmith",
        params: &[
            "LANGSMITH_API_KEY",
            "LANGSMITH_PROJECT",
            "LANGSMITH_DEFAULT_RUN_NAME",
        ],
    },
    AvailableCallback {
        key: "lago",
        litellm_callback_name: "lago",
        ui_callback_name: "Lago Billing",
        params: &[
            "LAGO_API_BASE",
            "LAGO_API_KEY",
            "LAGO_API_EVENT_CODE",
            "LAGO_API_CHARGE_BY",
        ],
    },
    AvailableCallback {
        key: "traceloop",
        litellm_callback_name: "traceloop",
        ui_callback_name: "Traceloop",
        params: &["TRACELOOP_API_KEY"],
    },
    AvailableCallback {
        key: "galileo",
        litellm_callback_name: "galileo",
        ui_callback_name: "Galileo",
        params: &[
            "GALILEO_API_KEY",
            "GALILEO_PROJECT_ID",
            "GALILEO_LOG_STREAM_ID",
            "GALILEO_BASE_URL",
            "GALILEO_USERNAME",
            "GALILEO_PASSWORD",
        ],
    },
    AvailableCallback {
        key: "newrelic",
        litellm_callback_name: "newrelic",
        ui_callback_name: "New Relic",
        params: &["NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED"],
    },
    AvailableCallback {
        key: "pointfive",
        litellm_callback_name: "pointfive",
        ui_callback_name: "PointFive",
        params: &["POINTFIVE_API_KEY", "POINTFIVE_API_URL"],
    },
    AvailableCallback {
        key: "signoz",
        litellm_callback_name: "signoz",
        ui_callback_name: "SigNoz",
        params: &["SIGNOZ_INGESTION_ENDPOINT", "SIGNOZ_INGESTION_KEY"],
    },
    AvailableCallback {
        key: "zerobus",
        litellm_callback_name: "zerobus",
        ui_callback_name: "Databricks Zerobus",
        params: &[
            "ZEROBUS_WORKSPACE_URL",
            "ZEROBUS_SERVER_ENDPOINT",
            "ZEROBUS_CLIENT_ID",
            "ZEROBUS_CLIENT_SECRET",
            "ZEROBUS_TABLE_NAME",
        ],
    },
];

pub(crate) fn callback_env_vars(name: &str) -> &'static [&'static str] {
    let normalized = name.to_lowercase();
    let looked_up = match normalized.as_str() {
        "langfuse_otel" => "langfuse",
        "s3_v2" => "s3",
        other => other,
    };
    match AVAILABLE_CALLBACKS
        .iter()
        .find(|callback| callback.key == looked_up)
    {
        Some(callback) => callback.params,
        None => &[],
    }
}
