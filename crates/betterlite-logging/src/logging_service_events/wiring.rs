use std::sync::Arc;

use betterlite_config::sections::service_callback::Settings as ServiceCallbackSettings;
use betterlite_db::database_diagnostics::level::LogLevel;

use crate::callback_config::{CallbackVars, FeatureFlags};
use crate::logging_service_events::pipeline::{PipelineConfig, PipelineError, ServicePipeline};
use crate::logging_service_events::sink::ServiceEventSink;
use crate::logging_success::ports::{StreamAssembler, SuccessClock};
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackList;
use crate::worker::{DEFAULT_SHUTDOWN_DRAIN, DEFAULT_TIMEOUT_SUMMARY_WINDOW, WorkerSettings};

// The one place the server builds the logging parts: clock, sink, callback registry and stream assembler.
pub struct LoggingWiring {
    pub pipeline: Arc<ServicePipeline>,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    // the limits the worker was started with, resolved from the config and the environment
    pub worker_settings: WorkerSettings,
    pub log_level: LogLevel,
    pub json_logs: bool,
}

impl LoggingWiring {
    pub fn start(
        settings: &ServiceCallbackSettings,
        clock: Arc<dyn SuccessClock>,
        sink: Arc<dyn ServiceEventSink>,
        mut registry: CallbackRegistry,
        stream_assembler: Arc<dyn StreamAssembler>,
    ) -> Result<Self, PipelineError> {
        register_service_callbacks(&mut registry, &settings.service_callbacks);
        let worker_settings = WorkerSettings {
            max_queue_size: settings.max_queue_size,
            concurrency: settings.concurrency,
            event_timeout: settings.event_timeout,
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            timeout_summary_window: DEFAULT_TIMEOUT_SUMMARY_WINDOW,
        };
        let pipeline = ServicePipeline::start(PipelineConfig {
            settings: worker_settings,
            clock,
            sink,
            registry: Arc::new(registry),
        })?;
        Ok(Self {
            pipeline: Arc::new(pipeline),
            stream_assembler,
            worker_settings,
            log_level: settings.log_level,
            json_logs: settings.json_logs,
        })
    }
}

// A callback reaches the service phase only once its adapter exists; until then it is reported as not installed.
fn register_service_callbacks(registry: &mut CallbackRegistry, names: &[String]) {
    for name in names {
        let registered = registry.register_configured(
            name,
            &CallbackVars::new(),
            CallbackList::Service,
            FeatureFlags {
                otel_v2_enabled: false,
            },
        );
        match registered {
            Ok(registration) => tracing::debug!(
                service_callback = %name,
                ?registration,
                "service callback registration"
            ),
            Err(error) => tracing::warn!(
                service_callback = %name,
                %error,
                "service callback is not installed"
            ),
        }
    }
}
