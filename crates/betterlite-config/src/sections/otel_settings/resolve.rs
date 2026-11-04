use super::{ExporterSpec, ListSetting, Raw, RawOtel, Settings};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut raw = self
            .callback_settings
            .and_then(|settings| settings.otel)
            .unwrap_or_default();
        references::resolve(&mut raw, lookup)?;
        raw.exporters = structured_setting(lookup, "exporters", raw.exporters.take())?;
        raw.resource_attributes = structured_setting(
            lookup,
            "resource_attributes",
            raw.resource_attributes.take(),
        )?;
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

struct CoreSettings {
    shorthand_explicit: bool,
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
    fn resolve<L>(raw: &RawOtel, lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let [configured_exporter, endpoint, traces_endpoint, headers] = shorthand(raw, lookup);
        let (service_name, deployment_environment) = service_identity(raw, lookup);
        let excluded_services = resolve_exclusions(raw, lookup);
        let features = FeatureSettings::resolve(raw, lookup)?;
        let shorthand_explicit =
            configured_exporter.is_some() || endpoint.is_some() || headers.is_some();
        let exporter = configured_exporter.unwrap_or_else(|| "console".to_owned());
        Ok(Self {
            shorthand_explicit,
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
    fn resolve<L>(raw: &RawOtel, lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
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
            capture_message_content: environment_or(
                lookup,
                &["OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT"],
                raw.capture_message_content.clone(),
                |value| value,
            )
            .unwrap_or_else(|| "no_content".to_owned())
            .to_lowercase(),
            legacy_compat: setting_bool(
                lookup,
                "LITELLM_OTEL_LEGACY_COMPAT",
                raw.legacy_compat,
                true,
            )?,
            langfuse_span_scope: environment_or(
                lookup,
                &["langfuse_span_scope", "LITELLM_OTEL_LANGFUSE_SPAN_SCOPE"],
                raw.langfuse_span_scope.clone(),
                |value| value,
            )
            .unwrap_or_else(|| "full".to_owned())
            .trim()
            .to_lowercase(),
        })
    }
}

fn resolve_baggage<L>(raw: &RawOtel, lookup: &L) -> (Vec<String>, Vec<String>, Vec<String>)
where
    L: EnvironmentLookup,
{
    (
        environment_or(
            lookup,
            &[
                "baggage_promoted_keys",
                "LITELLM_OTEL_BAGGAGE_PROMOTED_KEYS",
            ],
            raw.baggage_promoted_keys.clone().map(ListSetting::values),
            |value| csv_values(&value),
        )
        .unwrap_or_else(|| default_baggage_keys(DEFAULT_BAGGAGE_PROMOTED_KEYS)),
        environment_or(
            lookup,
            &[
                "baggage_metadata_keys",
                "LITELLM_OTEL_BAGGAGE_METADATA_KEYS",
            ],
            raw.baggage_metadata_keys.clone().map(ListSetting::values),
            |value| csv_values(&value),
        )
        .unwrap_or_else(|| default_baggage_keys(DEFAULT_BAGGAGE_METADATA_KEYS)),
        environment_or(
            lookup,
            &[
                "baggage_team_metadata_keys",
                "LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS",
            ],
            raw.baggage_team_metadata_keys
                .clone()
                .map(ListSetting::values),
            |value| csv_values(&value),
        )
        .unwrap_or_default(),
    )
}

fn resolve_mapper_names<L>(raw: &RawOtel, lookup: &L, legacy_compat: bool) -> Vec<String>
where
    L: EnvironmentLookup,
{
    let mapper_names = environment_or(
        lookup,
        &["mapper_names", "MAPPER_NAMES"],
        raw.mapper_names.clone().map(ListSetting::values),
        |value| csv_values(&value),
    )
    .unwrap_or_else(|| vec!["genai".to_owned()]);
    let mut names = vec!["genai".to_owned()];
    for name in mapper_names {
        if name != "genai" {
            names.push(name);
        }
    }
    if legacy_compat && !names.iter().any(|name| name == "legacy") {
        names.push("legacy".to_owned());
    }
    names
}

fn resolve_exporters(raw: &RawOtel, core: &CoreSettings) -> Vec<ExporterSpec> {
    if let Some(exporters) = raw
        .exporters
        .as_ref()
        .filter(|exporters| !exporters.is_empty())
    {
        return exporters.clone();
    }
    if !core.shorthand_explicit {
        return vec![ExporterSpec::default()];
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
    environment_or, resolve_exclusions, service_identity, setting_bool, shorthand,
    structured_setting,
};

mod references;
