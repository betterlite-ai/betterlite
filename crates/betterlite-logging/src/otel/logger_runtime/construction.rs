use std::sync::Arc;

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use betterlite_redact::secret::Redactor;
use chrono::FixedOffset;

use crate::otel::content_projections::StreamOutputAssembler;
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
use super::types::{
    BuiltLogger, CallbackHooks, LoggerBuildRequest, LoggerClass, V2LoggerConstructor,
};

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
        let parts = callback_parts(self, request)?;
        let assembly = V2Callback::assemble(parts, request.settings)?;
        built_logger(self, request, assembly)
    }
}

fn callback_parts(
    constructor: &CallbackConstructor,
    request: &LoggerBuildRequest<'_>,
) -> Result<CallbackParts, LoggerRuntimeError> {
    Ok(CallbackParts {
        callback_name: request.callback_name.map(str::to_owned),
        config: request.config.clone(),
        raw_config: constructor.raw_config.clone(),
        tenants: tenant_cache(constructor, request),
        trace: Arc::clone(&request.trace),
        // an injected provider is used as given; only the recorders a build creates start unset
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
            clients: request.factories.clients.clone(),
        },
        metric: MetricSettings {
            callback_settings: Arc::clone(&constructor.callback_settings),
            utc_offset: constructor.utc_offset,
        },
        emitter: emitter_parts(constructor, request)?,
        error_info: ErrorInformationConfig {
            normalizer: Normalizer::new()?,
            redactor: constructor.redactor.clone(),
            log_client_error_tracebacks: constructor.log_client_error_tracebacks,
        },
    })
}

fn tenant_cache(
    constructor: &CallbackConstructor,
    request: &LoggerBuildRequest<'_>,
) -> TenantTracerCache {
    TenantTracerCache::new(
        request.config.clone(),
        request.callback_name.map(str::to_owned),
        TRACER_NAME,
        TenantSources {
            factories: Arc::clone(&request.factories),
            scope_source: Arc::clone(&request.scope_source),
            allowed_hosts: Arc::clone(&constructor.allowed_hosts),
        },
    )
}

fn emitter_parts(
    constructor: &CallbackConstructor,
    request: &LoggerBuildRequest<'_>,
) -> Result<EmitterParts, LoggerRuntimeError> {
    let mappers = resolve_mappers(&request.config.mapper_names)
        .map_err(|source| LoggerRuntimeError::Constructor(CallbackError::Mapper(source)))?
        .into_iter()
        .map(MapperKind::from)
        .collect();
    Ok(EmitterParts {
        mappers,
        environment: Arc::clone(&request.environment),
        attribute_limit: constructor.attribute_limit,
    })
}

fn built_logger(
    constructor: &CallbackConstructor,
    request: &LoggerBuildRequest<'_>,
    assembly: V2CallbackAssembly,
) -> Result<BuiltLogger, LoggerRuntimeError> {
    let roots = Arc::clone(&constructor.roots);
    match request.class {
        LoggerClass::Generic => Ok(generic_built(assembly)),
        LoggerClass::Langfuse => Ok(langfuse_built(assembly, roots, None)),
        LoggerClass::LangfuseContent => {
            let Some(assembler) = request.assembler else {
                return Err(LoggerRuntimeError::MissingAssembler);
            };
            Ok(langfuse_built(assembly, roots, Some(assembler)))
        }
    }
}

fn generic_built(assembly: V2CallbackAssembly) -> BuiltLogger {
    let V2CallbackAssembly {
        provider,
        logger,
        recorder_providers,
        error_info,
    } = assembly;
    let typed = Arc::new(V2Callback {
        tracer_provider: provider.clone(),
        recorder_providers: recorder_providers.clone(),
        logger,
        roots: RootSpans::default(),
        error_info,
    });
    let hooks = Some(CallbackHooks::Generic(Arc::clone(&typed)));
    BuiltLogger {
        callback: typed,
        hooks,
        provider,
        recorder_providers,
    }
}

// B8250's wrapper owns the assembled logger and keeps the request root span source
fn langfuse_built(
    assembly: V2CallbackAssembly,
    roots: Arc<dyn RootSpanSource>,
    stream_assembler: Option<&Arc<dyn StreamOutputAssembler>>,
) -> BuiltLogger {
    let V2CallbackAssembly {
        provider,
        logger,
        recorder_providers,
        error_info: _,
    } = assembly;
    let inner = LangfuseOtel {
        inner: logger,
        roots,
    };
    if let Some(assembler) = stream_assembler {
        let typed = Arc::new(LangfuseContentOtel {
            inner,
            stream_assembler: Arc::clone(assembler),
        });
        let hooks = Some(CallbackHooks::LangfuseContent(Arc::clone(&typed)));
        return BuiltLogger {
            callback: typed,
            hooks,
            provider,
            recorder_providers,
        };
    }
    let typed = Arc::new(inner);
    let hooks = Some(CallbackHooks::Langfuse(Arc::clone(&typed)));
    BuiltLogger {
        callback: typed,
        hooks,
        provider,
        recorder_providers,
    }
}
