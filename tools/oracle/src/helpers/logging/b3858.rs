use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::legacy_lifecycle::lifecycle::{
    DEFAULT_HANDLER_CLASS_NAME, LegacyLifecycle, LegacyLifecycleOptions,
};
use betterlite_logging::otel::legacy_lifecycle::tracer::BaseDynamicOtel;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::plumbing::events::SystemClock;
use betterlite_logging::otel::v2_callback::live_span::RootSpans;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use serde::Deserialize;

mod calls;
mod harness;
mod observe;
mod tests;
use calls::{
    end_parent, error_attributes_call, failure_call, management_call, service_call, success_call,
};
use harness::prepare;
use observe::observed;

pub(crate) struct Op;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Input {
    function: String,
    start_time: f64,
    end_time: f64,
    parent: Option<ParentInput>,
    service: String,
    call_type: String,
    caller: Option<String>,
    error: Option<String>,
    event_metadata: Option<MetadataMap>,
    kwargs: MetadataMap,
    request_data: MetadataMap,
    exception: Option<ExceptionInput>,
    team: Option<TeamInput>,
    after_failure_log: bool,
    route: String,
    response: Option<MetadataMap>,
    status_code: i64,
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ParentInput {
    name: String,
    start: Option<f64>,
    end: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ExceptionInput {
    kind: String,
    message: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TeamInput {
    id: Option<String>,
    alias: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let exporter = InMemorySpanExporter::default();
        let roots = Arc::new(RootSpans::default());
        let (otel, parent) = prepare(input, &exporter, &roots).or_value_error()?;
        let lifecycle = LegacyLifecycle::from_otel(
            otel,
            LegacyLifecycleOptions {
                dynamic: Arc::new(BaseDynamicOtel),
                live: Arc::clone(&roots),
                metrics: None,
                events: None,
                failure_counters: Vec::new(),
                handler_class_name: DEFAULT_HANDLER_CLASS_NAME.to_owned(),
                use_request_span: false,
                clock: Box::new(SystemClock),
            },
        );
        let outcome = match input.function.as_str() {
            "service_success" | "service_failure" => {
                service_call(&lifecycle, input, parent.as_ref())
            }
            "post_call_failure" => failure_call(&lifecycle, input, parent.as_ref()),
            "post_call_success" => success_call(&lifecycle, input, parent.as_ref()),
            "management_success" | "management_failure" => {
                management_call(&lifecycle, input, parent.as_ref())
            }
            "error_attributes" => error_attributes_call(input, parent.as_ref()),
            other => Err(format!("unknown function {other}")),
        };
        outcome.or_value_error()?;
        end_parent(input, parent.as_ref());
        observed(&exporter).or_value_error()
    }
}
