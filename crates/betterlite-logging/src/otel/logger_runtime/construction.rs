use std::sync::Arc;

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use betterlite_redact::secret::Redactor;
use chrono::FixedOffset;

use crate::callback_logger::CallbackLogger;
use crate::otel::emitter::span_emission::MapperKind;
use crate::otel::langfuse_root::{LangfuseContentOtel, LangfuseOtel, RootSpanSource};
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::mappers::registry::resolve_mappers;
use crate::otel::plumbing::providers::ProviderState;
use crate::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use crate::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
    V2CallbackAssembly,
};
use crate::otel::v2_callback::error::CallbackError;
use crate::otel::v2_callback::live_span::RootSpans;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

use super::error::LoggerRuntimeError;
use super::types::{BuiltLogger, LoggerBuildRequest, LoggerClass, V2LoggerConstructor};

const TRACER_NAME: &str = "litellm";

// build_otel_v2_logger's constructor argument: generic is B3853's callback, the Langfuse classes are B8250's wrappers
pub struct CallbackConstructor {
    pub raw_config: otel_settings::Settings,
    pub callback_settings: Arc<dyn CallbackSettingsView>,
    pub allowed_hosts: Arc<[AllowedDestination]>,
    pub roots: Arc<dyn RootSpanSource>,
    pub attribute_limit: Option<usize>,
    pub redactor: Option<Arc<Redactor>>,
    pub log_client_error_tracebacks: bool,
    pub utc_offset: FixedOffset,
}

impl V2LoggerConstructor for CallbackConstructor {
    fn build(&self, request: &LoggerBuildRequest<'_>) -> Result<BuiltLogger, LoggerRuntimeError> {
        let callback_name = request.callback_name.map(str::to_owned);
        let mappers = resolve_mappers(&request.config.mapper_names)
            .map_err(|source| LoggerRuntimeError::Constructor {
                source: CallbackError::Mapper(source),
            })?
            .into_iter()
            .map(MapperKind::from)
            .collect();
        let parts = CallbackParts {
            callback_name: callback_name.clone(),
            config: request.config.clone(),
            raw_config: self.raw_config.clone(),
            tenants: TenantTracerCache::new(
                request.config.clone(),
                callback_name,
                TRACER_NAME,
                TenantSources {
                    factories: Arc::clone(&request.factories),
                    scope_source: Arc::clone(&request.scope_source),
                    allowed_hosts: Arc::clone(&self.allowed_hosts),
                },
            ),
            trace: Arc::clone(&request.trace),
            providers: ProviderSources {
                tracer_provider: request.providers.tracer_provider.cloned(),
                meter_provider: request.providers.meter_provider.cloned(),
                logger_provider: request.providers.logger_provider.cloned(),
                meter_state: ProviderState::Unset,
                logger_state: ProviderState::Unset,
                metrics: None,
                events: None,
            },
            build: ProviderBuild {
                factories: Arc::clone(&request.factories),
                scope_source: Arc::clone(&request.scope_source),
                clients: request.factories.clients(),
            },
            metric: MetricSettings {
                callback_settings: Arc::clone(&self.callback_settings),
                utc_offset: self.utc_offset,
            },
            emitter: EmitterParts {
                mappers,
                environment: Arc::clone(&request.environment),
                attribute_limit: self.attribute_limit,
            },
            error_info: ErrorInformationConfig {
                normalizer: Normalizer::new()
                    .map_err(|source| LoggerRuntimeError::Normalizer { source })?,
                redactor: self.redactor.clone(),
                log_client_error_tracebacks: self.log_client_error_tracebacks,
            },
        };
        let V2CallbackAssembly {
            provider,
            logger,
            recorder_providers,
            error_info,
        } = V2Callback::assemble(parts)
            .map_err(|source| LoggerRuntimeError::Constructor { source })?;
        let callback: Arc<dyn CallbackLogger> = match request.class {
            LoggerClass::Generic => Arc::new(V2Callback {
                tracer_provider: provider.clone(),
                recorder_providers: recorder_providers.clone(),
                logger,
                roots: RootSpans::default(),
                error_info,
            }),
            LoggerClass::Langfuse => Arc::new(LangfuseOtel {
                inner: logger,
                roots: Arc::clone(&self.roots),
            }),
            LoggerClass::LangfuseContent => {
                let Some(assembler) = request.assembler else {
                    return Err(LoggerRuntimeError::MissingAssembler);
                };
                Arc::new(LangfuseContentOtel {
                    inner: LangfuseOtel {
                        inner: logger,
                        roots: Arc::clone(&self.roots),
                    },
                    stream_assembler: Arc::clone(assembler),
                })
            }
        };
        Ok(BuiltLogger {
            callback,
            provider,
            recorder_providers,
        })
    }
}
