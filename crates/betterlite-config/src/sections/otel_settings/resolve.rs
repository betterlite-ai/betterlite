use super::{ExporterSpec, ListSetting, Raw, Settings};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = merge_callback_settings(self);
        let core = CoreSettings::resolve(&raw, lookup)?;
        let exporters = resolve_exporters(&raw, &core);
        let mapper_names = resolve_mapper_names(&raw, lookup, core.legacy_compat);
        let (baggage_promoted_keys, baggage_metadata_keys, baggage_team_metadata_keys) =
            resolve_baggage(&raw, lookup);
        Ok(Settings {
            exporter: core.exporter,
            endpoint: core.endpoint,
            traces_endpoint: core.traces_endpoint,
            headers: core.headers,
            service_name: core.service_name,
            deployment_environment: core.deployment_environment,
            enable_metrics: core.enable_metrics,
            enable_events: core.enable_events,
            capture_message_content: core.capture_message_content,
            legacy_compat: core.legacy_compat,
            langfuse_span_scope: core.langfuse_span_scope,
            excluded_services: core.excluded_services,
            exporters,
            mapper_names,
            resource_attributes: raw.resource_attributes.unwrap_or_default(),
            baggage_promoted_keys,
            baggage_metadata_keys,
            baggage_team_metadata_keys,
        })
    }
}

fn merge_callback_settings(mut raw: Raw) -> Raw {
    if let Some(callback) = raw
        .callback_settings
        .take()
        .and_then(|settings| settings.otel)
    {
        raw.exporter = raw.exporter.or(callback.exporter);
        raw.endpoint = raw.endpoint.or(callback.endpoint);
        raw.traces_endpoint = raw.traces_endpoint.or(callback.traces_endpoint);
        raw.headers = raw.headers.or(callback.headers);
        raw.service_name = raw.service_name.or(callback.service_name);
        raw.deployment_environment = raw
            .deployment_environment
            .or(callback.deployment_environment);
        raw.enable_metrics = raw.enable_metrics.or(callback.enable_metrics);
        raw.enable_events = raw.enable_events.or(callback.enable_events);
        raw.capture_message_content = raw
            .capture_message_content
            .or(callback.capture_message_content);
        raw.legacy_compat = raw.legacy_compat.or(callback.legacy_compat);
        raw.langfuse_span_scope = raw.langfuse_span_scope.or(callback.langfuse_span_scope);
        raw.excluded_services = raw.excluded_services.or(callback.excluded_services);
        raw.exporters = raw.exporters.or(callback.exporters);
        raw.mapper_names = raw.mapper_names.or(callback.mapper_names);
        raw.resource_attributes = raw.resource_attributes.or(callback.resource_attributes);
        raw.baggage_promoted_keys = raw.baggage_promoted_keys.or(callback.baggage_promoted_keys);
        raw.baggage_metadata_keys = raw.baggage_metadata_keys.or(callback.baggage_metadata_keys);
        raw.baggage_team_metadata_keys = raw
            .baggage_team_metadata_keys
            .or(callback.baggage_team_metadata_keys);
    }
    raw
}

struct CoreSettings {
    exporter: String,
    endpoint: Option<String>,
    traces_endpoint: Option<String>,
    headers: Option<String>,
    service_name: String,
    deployment_environment: Option<String>,
    enable_metrics: bool,
    enable_events: bool,
    capture_message_content: String,
    legacy_compat: bool,
    langfuse_span_scope: String,
    excluded_services: Vec<String>,
}

impl CoreSettings {
    fn resolve<L: EnvironmentLookup>(raw: &Raw, lookup: &L) -> Result<Self, ConfigError> {
        let exporter = environment_or(
            lookup,
            &["OTEL_EXPORTER", "OTEL_EXPORTER_OTLP_PROTOCOL"],
            raw.exporter.clone(),
            |value| value,
        )
        .unwrap_or_else(|| "console".to_owned());
        let endpoint = environment_or(
            lookup,
            &["OTEL_ENDPOINT", "OTEL_EXPORTER_OTLP_ENDPOINT"],
            raw.endpoint.clone(),
            |value| value,
        );
        let traces_endpoint = environment_or(
            lookup,
            &["OTEL_TRACES_ENDPOINT", "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"],
            raw.traces_endpoint.clone(),
            |value| value,
        );
        let headers = environment_or(
            lookup,
            &["OTEL_HEADERS", "OTEL_EXPORTER_OTLP_HEADERS"],
            raw.headers.clone(),
            |value| value,
        );
        let service_name = environment_or(
            lookup,
            &["OTEL_SERVICE_NAME"],
            raw.service_name.clone(),
            |value| value,
        )
        .unwrap_or_else(|| "litellm".to_owned());
        let deployment_environment = environment_or(
            lookup,
            &["OTEL_ENVIRONMENT_NAME"],
            raw.deployment_environment.clone(),
            |value| value,
        );
        let excluded_services = normalized_exclusions(environment_or(
            lookup,
            &["LITELLM_OTEL_EXCLUDED_SERVICES"],
            raw.excluded_services.clone(),
            serde_json::Value::String,
        ));
        let features = FeatureSettings::resolve(raw, lookup)?;
        Ok(Self {
            exporter,
            endpoint,
            traces_endpoint,
            headers,
            service_name,
            deployment_environment,
            enable_metrics: features.enable_metrics,
            enable_events: features.enable_events,
            capture_message_content: features.capture_message_content,
            legacy_compat: features.legacy_compat,
            langfuse_span_scope: features.langfuse_span_scope,
            excluded_services,
        })
    }
}

struct FeatureSettings {
    enable_metrics: bool,
    enable_events: bool,
    capture_message_content: String,
    legacy_compat: bool,
    langfuse_span_scope: String,
}

impl FeatureSettings {
    fn resolve<L: EnvironmentLookup>(raw: &Raw, lookup: &L) -> Result<Self, ConfigError> {
        Ok(Self {
            enable_metrics: setting_bool(
                lookup,
                "LITELLM_OTEL_INTEGRATION_ENABLE_METRICS",
                raw.enable_metrics,
                false,
            )?,
            enable_events: setting_bool(
                lookup,
                "LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS",
                raw.enable_events,
                false,
            )?,
            capture_message_content: lookup
                .lookup("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT")
                .or_else(|| raw.capture_message_content.clone())
                .unwrap_or_else(|| "no_content".to_owned())
                .to_lowercase(),
            legacy_compat: setting_bool(
                lookup,
                "LITELLM_OTEL_LEGACY_COMPAT",
                raw.legacy_compat,
                true,
            )?,
            langfuse_span_scope: lookup
                .lookup("LITELLM_OTEL_LANGFUSE_SPAN_SCOPE")
                .or_else(|| raw.langfuse_span_scope.clone())
                .unwrap_or_else(|| "full".to_owned())
                .trim()
                .to_lowercase(),
        })
    }
}

fn resolve_baggage<L: EnvironmentLookup>(
    raw: &Raw,
    lookup: &L,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    (
        environment_or(
            lookup,
            &["LITELLM_OTEL_BAGGAGE_PROMOTED_KEYS"],
            raw.baggage_promoted_keys.clone().map(ListSetting::values),
            |value| csv_values(&value),
        )
        .unwrap_or_else(|| default_baggage_keys(DEFAULT_BAGGAGE_PROMOTED_KEYS)),
        environment_or(
            lookup,
            &["LITELLM_OTEL_BAGGAGE_METADATA_KEYS"],
            raw.baggage_metadata_keys.clone().map(ListSetting::values),
            |value| csv_values(&value),
        )
        .unwrap_or_else(|| default_baggage_keys(DEFAULT_BAGGAGE_METADATA_KEYS)),
        environment_or(
            lookup,
            &["LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS"],
            raw.baggage_team_metadata_keys
                .clone()
                .map(ListSetting::values),
            |value| csv_values(&value),
        )
        .unwrap_or_default(),
    )
}

fn resolve_mapper_names<L: EnvironmentLookup>(
    raw: &Raw,
    lookup: &L,
    legacy_compat: bool,
) -> Vec<String> {
    let mapper_names = environment_or(
        lookup,
        &["LITELLM_OTEL_MAPPER_NAMES"],
        raw.mapper_names.clone().map(ListSetting::values),
        |value| csv_values(&value),
    )
    .unwrap_or_else(|| vec!["genai".to_owned()]);
    let mut names = vec!["genai".to_owned()];
    for name in mapper_names {
        if name != "genai" && !names.contains(&name) {
            names.push(name);
        }
    }
    if legacy_compat && !names.iter().any(|name| name == "legacy") {
        names.push("legacy".to_owned());
    }
    names
}

fn resolve_exporters(raw: &Raw, core: &CoreSettings) -> Vec<ExporterSpec> {
    if let Some(exporters) = raw
        .exporters
        .as_ref()
        .filter(|exporters| !exporters.is_empty())
    {
        return exporters.clone();
    }
    let kind = if core.exporter == "console"
        && (core.endpoint.is_some() || core.traces_endpoint.is_some())
    {
        "otlp_http".to_owned()
    } else {
        core.exporter.clone()
    };
    vec![ExporterSpec {
        kind: Some(kind),
        endpoint: core.endpoint.clone(),
        traces_endpoint: core.traces_endpoint.clone(),
        headers: core.headers.clone(),
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: None,
    }]
}

mod helpers;
pub(super) use helpers::csv_values;
use helpers::{
    DEFAULT_BAGGAGE_METADATA_KEYS, DEFAULT_BAGGAGE_PROMOTED_KEYS, default_baggage_keys,
    environment_or, normalized_exclusions, setting_bool,
};
