use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{CaptureMessageContent, ExporterSpec, Settings};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaseInput {
    #[serde(default)]
    exporters: Vec<ExporterInput>,
    #[serde(default)]
    mapper_names: Vec<String>,
    #[serde(default)]
    resource_attributes: BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExporterInput {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    headers: Option<String>,
    #[serde(default)]
    requires_headers: bool,
}

impl BaseInput {
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            exporters: self.exporters.iter().map(ExporterInput::spec).collect(),
            service_name: "litellm".to_owned(),
            deployment_environment: None,
            enable_metrics: false,
            enable_events: false,
            capture_message_content: CaptureMessageContent::NoContent,
            legacy_compat: false,
            langfuse_span_scope: SpanScope::Full,
            excluded_services: BTreeSet::new(),
            mapper_names: self.mapper_names.clone(),
            resource_attributes: self.resource_attributes.clone(),
            baggage_promoted_keys: Vec::new(),
            baggage_metadata_keys: Vec::new(),
            baggage_team_metadata_keys: Vec::new(),
        }
    }
}

impl ExporterInput {
    fn spec(&self) -> ExporterSpec {
        ExporterSpec {
            kind: self.kind.clone().unwrap_or_else(|| "console".to_owned()),
            endpoint: self.endpoint.clone(),
            traces_endpoint: None,
            headers: self.headers.clone(),
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: self.requires_headers,
        }
    }
}
