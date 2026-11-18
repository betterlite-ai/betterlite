use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_logging::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use betterlite_logging::otel::legacy_lifecycle::context::instant_from_ns;
use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use betterlite_logging::otel::legacy_policy::span_time::{SpanTime, to_ns};
use betterlite_logging::otel::plumbing::exporters::HttpClients;
use betterlite_logging::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use betterlite_logging::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use betterlite_logging::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use opentelemetry::Context;
use opentelemetry::trace::Tracer as _;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::{Input, ParentInput};

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no connection is opened by this helper"))
    }
}

const fn config() -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: Exporter::InMemory,
        endpoint: None,
        headers: None,
        signals: Signals {
            enable_metrics: false,
            enable_events: false,
        },
        service_name: String::new(),
        deployment_environment: String::new(),
        model_id: String::new(),
        ignore_context_propagation: false,
        skip_set_global: true,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

pub(super) fn prepare(
    input: &Input,
    exporter: &InMemorySpanExporter,
    roots: &RootSpans,
) -> Result<(LegacyOtel, Option<LiveSpan>, Option<LiveSpan>), String> {
    let client: Arc<dyn HttpClient> = Arc::new(NoServer);
    let otel = init_legacy_otel(
        LegacyOtelInit {
            config: Some(config()),
            callback_name: Some("opentelemetry".to_owned()),
            overrides: MetadataKeyOverrides::default(),
            injected: InjectedProviders::default(),
            max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
        },
        &LegacyOtelEnvironment {
            env: &input.environment,
            clients: HttpClients {
                traces: Arc::clone(&client),
                metrics: Arc::clone(&client),
                logs: client,
            },
            shared_exporter: exporter.clone(),
            globals: &GlobalProviders::default(),
            proxy: None,
        },
    )
    .map_err(|error| error.to_string())?;
    let parent = build_span(&otel, input.parent.as_ref(), input.start_time, roots);
    let key_parent = build_span(&otel, input.key_parent.as_ref(), input.start_time, roots);
    Ok((otel, parent, key_parent))
}

fn build_span(
    otel: &LegacyOtel,
    spec: Option<&ParentInput>,
    default_start: f64,
    roots: &RootSpans,
) -> Option<LiveSpan> {
    let spec = spec?;
    let start = spec.start.unwrap_or(default_start);
    let builder = otel
        .tracer
        .span_builder(spec.name.clone())
        .with_start_time(instant_from_ns(to_ns(SpanTime::Seconds(start), 0)));
    let live = LiveSpan::from(otel.tracer.build_with_context(builder, &Context::new()));
    roots.register(&live);
    Some(live)
}
