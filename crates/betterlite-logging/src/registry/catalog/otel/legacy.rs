use std::sync::Arc;

use chrono::Utc;
use opentelemetry::logs::LoggerProvider as _;

use super::super::{CatalogError, CatalogInputs};
use super::attributes::AttributeSettings;
use super::env;
use super::shared::SharedOtel;
use crate::callback_logger::CallbackLogger;
use crate::otel::legacy_lifecycle::lifecycle::{LegacyLifecycle, LegacyLifecycleOptions};
use crate::otel::legacy_lifecycle::logger::LegacyOtelLogger;
use crate::otel::legacy_metrics::events::SemanticEventEmitter;
use crate::otel::legacy_metrics::filter::{CallbackSettingsView, MetricFilterState};
use crate::otel::legacy_metrics::recorder::LegacyMetricRecorder;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::legacy_policy::semconv_attrs::semconv_latest_experimental;
use crate::otel::plumbing::events::SystemClock;
use crate::otel_provider_assembly::legacy_otel_init::{
    LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, init_legacy_otel,
};

// A legacy logger part-way through construction: the provider side is built, the lifecycle is not.
pub struct Prepared<'shared> {
    pub otel: LegacyOtel,
    view: Arc<dyn CallbackSettingsView>,
    shared: &'shared SharedOtel,
}

// B3868's init, after the metric filter this config names has been validated (opentelemetry.py 395-415).
pub fn prepare<'shared>(
    inputs: &CatalogInputs,
    shared: &'shared SharedOtel,
    config: OpenTelemetryConfig,
    callback_name: Option<&str>,
) -> Result<Prepared<'shared>, CatalogError> {
    let view = view(inputs);
    let _checked = MetricFilterState::new(
        config.attributes.clone(),
        callback_name.map(str::to_owned),
        Arc::clone(&view),
    )
    .map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    let otel = init(inputs, shared, config, callback_name)?;
    Ok(Prepared { otel, view, shared })
}

// B3910 builds the config and the LegacyOtel itself; the recorders and the filter are the same here.
pub fn complete<'shared>(
    inputs: &CatalogInputs,
    shared: &'shared SharedOtel,
    otel: LegacyOtel,
) -> Result<Prepared<'shared>, CatalogError> {
    let view = view(inputs);
    let _checked = MetricFilterState::new(
        otel.config.attributes.clone(),
        otel.callback_name.clone(),
        Arc::clone(&view),
    )
    .map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    Ok(Prepared { otel, view, shared })
}

fn view(inputs: &CatalogInputs) -> Arc<dyn CallbackSettingsView> {
    let settings = &inputs.config.sections.otel_settings;
    Arc::new(AttributeSettings::from_settings(settings))
}

fn init(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
    config: OpenTelemetryConfig,
    callback_name: Option<&str>,
) -> Result<LegacyOtel, CatalogError> {
    let clients = shared.clients(inputs)?;
    let environment = LegacyOtelEnvironment {
        env: &shared.env,
        clients,
        shared_exporter: shared.shared_exporter.clone(),
        globals: &shared.globals,
        proxy: Some(&shared.proxy),
    };
    let init = LegacyOtelInit {
        config: Some(config),
        callback_name: callback_name.map(str::to_owned),
        ..LegacyOtelInit::default()
    };
    init_legacy_otel(init, &environment).map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })
}

impl Prepared<'_> {
    // the metric recorder and the semantic event emitter the lifecycle options carry, from this logger's providers
    pub fn options(&mut self) -> Result<LegacyLifecycleOptions, CatalogError> {
        let mut options = LegacyLifecycleOptions::default();
        if let Some(signal) = self.otel.metrics.take() {
            let recorder = LegacyMetricRecorder::new(
                signal.instruments,
                &self.otel.config,
                self.otel.callback_name.clone(),
                Arc::clone(&self.view),
                // the V2 constructor takes its utc_offset the same way, so a legacy metric uses UTC
                Utc.fix(),
            )
            .map_err(|source| CatalogError::Init {
                message: source.to_string(),
            })?;
            self.shared.keep(Some(signal.meter_provider), None);
            options.metrics = Some(recorder);
        }
        if let Some(provider) = self.otel.logger_provider.take() {
            let event_logger = provider.logger(env::logger_name(&self.shared.env));
            self.shared.keep(None, Some(provider));
            options.events = Some(SemanticEventEmitter {
                event_logger,
                clock: Box::new(SystemClock),
                enable_events: self.otel.config.signals.enable_events,
                semconv_latest: semconv_latest_experimental(&self.otel.config),
            });
        }
        Ok(options)
    }
}

// The plain legacy OpenTelemetry: otel, logfire, langtrace, signoz and levo add no dynamic hooks.
pub fn plain(mut prepared: Prepared<'_>) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let options = prepared.options()?;
    Ok(lifecycle_logger(LegacyLifecycle::from_otel(
        prepared.otel,
        options,
    )))
}

pub fn lifecycle_logger(lifecycle: LegacyLifecycle) -> Arc<dyn CallbackLogger> {
    let held = Arc::new(lifecycle);
    Arc::new(LegacyOtelLogger::new(held))
}
