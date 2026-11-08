use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{
    CaptureMessageContent, ExporterOwner, ExporterSpec, Settings,
};

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
    traces_endpoint: Option<String>,
    #[serde(default)]
    headers: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    options: Option<BTreeMap<String, String>>,
    #[serde(default)]
    use_simple_processor: Option<bool>,
    #[serde(default)]
    requires_headers: bool,
}

impl BaseInput {
    pub(crate) fn settings(&self) -> Result<Settings, Box<crate::helpers::HelperError>> {
        let exporters = self
            .exporters
            .iter()
            .map(ExporterInput::spec)
            .collect::<Result<Vec<_>, _>>()?;
        let mut settings = fixed_settings();
        settings.exporters = exporters;
        self.mapper_names.clone_into(&mut settings.mapper_names);
        self.resource_attributes
            .clone_into(&mut settings.resource_attributes);
        Ok(settings)
    }
}

fn fixed_settings() -> Settings {
    Settings {
        exporters: Vec::new(),
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

impl ExporterInput {
    fn spec(&self) -> Result<ExporterSpec, Box<crate::helpers::HelperError>> {
        let owner = match self.owner.as_deref() {
            None => None,
            Some("arize") => Some(ExporterOwner::ArizeAx),
            Some("arize_phoenix") => Some(ExporterOwner::ArizePhoenix),
            Some("langfuse_otel") => Some(ExporterOwner::LangfuseOtel),
            Some("weave_otel") => Some(ExporterOwner::WeaveOtel),
            Some("levo") => Some(ExporterOwner::Levo),
            Some("agentops") => Some(ExporterOwner::Agentops),
            Some("newrelic") => Some(ExporterOwner::Newrelic),
            Some("signoz") => Some(ExporterOwner::Signoz),
            Some(unknown) => return Err(invalid(&format!("unknown exporter owner {unknown}"))),
        };
        Ok(ExporterSpec {
            kind: self.kind.clone().unwrap_or_else(|| "console".to_owned()),
            endpoint: self.endpoint.clone(),
            traces_endpoint: self.traces_endpoint.clone(),
            headers: self.headers.clone(),
            owner,
            options: self.options.clone(),
            use_simple_processor: self.use_simple_processor,
            requires_headers: self.requires_headers,
        })
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
