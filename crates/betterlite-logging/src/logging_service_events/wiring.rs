use std::sync::Arc;

use betterlite_config::sections::service_callback::SettingsValues;
use betterlite_db::database_diagnostics::level::LogLevel;

use crate::callback_config::{CallbackVars, FeatureFlags};
use crate::logging_service_events::pipeline::{PipelineConfig, PipelineError, ServicePipeline};
use crate::logging_service_events::sink::ServiceEventSink;
use crate::logging_success::owned::OwnedSuccessHandler;
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::payload::builder::{PayloadBuilder, PayloadBuilderConfig};
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackList;
use crate::worker::{DEFAULT_SHUTDOWN_DRAIN, DEFAULT_TIMEOUT_SUMMARY_WINDOW, WorkerSettings};

// Everything the server hands the logging parts once, so the pipeline and the success handler share them
pub struct LoggingInputs<'config> {
    pub settings: &'config SettingsValues,
    pub clock: Arc<dyn SuccessClock>,
    pub sink: Arc<dyn ServiceEventSink>,
    pub registry: CallbackRegistry,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub success: SuccessInputs,
}

// What the one success handler needs beyond the registry, clock and assembler the pipeline shares
pub struct SuccessInputs {
    pub payload: PayloadBuilderConfig,
    pub excluded_fields: Vec<String>,
    pub known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

// The one place the server builds the logging parts: clock, sink, callback registry and stream assembler.
pub struct LoggingWiring {
    pub pipeline: Arc<ServicePipeline>,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub success: Arc<OwnedSuccessHandler>,
    // the limits the worker was started with, resolved from the config and the environment
    pub worker_settings: WorkerSettings,
    pub log_level: LogLevel,
    pub json_logs: bool,
}

impl LoggingWiring {
    pub fn start(inputs: LoggingInputs<'_>) -> Result<Self, PipelineError> {
        let LoggingInputs {
            settings,
            clock,
            sink,
            mut registry,
            stream_assembler,
            success,
        } = inputs;
        // an unknown LITELLM_LOG stops the server here, as it stops LiteLLM when _logging imports
        let log_level = settings.resolve_log_level()?;
        register_service_callbacks(&mut registry, &settings.service_callbacks);
        let worker_settings = WorkerSettings {
            max_queue_size: settings.max_queue_size,
            concurrency: settings.concurrency,
            event_timeout: settings.event_timeout,
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            timeout_summary_window: DEFAULT_TIMEOUT_SUMMARY_WINDOW,
        };
        // the registry is filled before the one Arc is made, so the handler's lookups never race a write
        let shared_registry = Arc::new(registry);
        let privacy = success.payload.privacy;
        let payload_builder: Arc<dyn StandardPayloadBuilder> =
            Arc::new(PayloadBuilder::new(success.payload));
        let success_handler = Arc::new(OwnedSuccessHandler {
            registry: Arc::clone(&shared_registry),
            payload_builder,
            stream_assembler: Arc::clone(&stream_assembler),
            clock: Arc::clone(&clock),
            excluded_fields: success.excluded_fields,
            known_callback: success.known_callback,
            privacy,
        });
        let pipeline = ServicePipeline::start(PipelineConfig {
            settings: worker_settings,
            clock,
            sink,
            registry: shared_registry,
        })?;
        Ok(Self {
            pipeline: Arc::new(pipeline),
            stream_assembler,
            success: success_handler,
            worker_settings,
            log_level,
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
