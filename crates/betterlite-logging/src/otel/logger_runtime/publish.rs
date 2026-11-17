use std::collections::BTreeSet;
use std::slice::from_ref;
use std::sync::Arc;

use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::callback_logger::CallbackLogger;
use crate::otel::config::Settings;
use crate::otel::fan_out::attach::attach_tenant_fan_out;

use super::error::LoggerRuntimeError;
use super::select::{excluded_db_systems, select_global_otel_v2_logger, v2_configs};
use super::types::{FanOutProvider, V2Logger};
use super::{LoggerRuntime, published};

// litellm.integrations.otel.logger.publish_global_otel_v2_provider: fan-out first, then the setter
pub fn publish_global_v2_provider(
    runtime: &LoggerRuntime,
    in_memory: &[Arc<dyn CallbackLogger>],
    set_global_provider: &dyn Fn(&SdkTracerProvider),
) -> Result<Arc<V2Logger>, LoggerRuntimeError> {
    let logger = select_global_otel_v2_logger(runtime, in_memory)?;
    let configs = v2_configs(runtime, in_memory, &logger);
    let excluded = excluded_db_systems(runtime, &logger.config);
    attach_fan_out(runtime, &configs, excluded);
    set_global_provider(&logger.provider);
    *published(runtime) = Some(logger.provider.clone());
    Ok(logger)
}

// litellm.integrations.otel.logger.fan_out_provider: published, then registered, then the SDK global
#[must_use]
pub fn fan_out_provider(runtime: &LoggerRuntime) -> FanOutProvider {
    let remembered = published(runtime).as_ref().cloned();
    if let Some(provider) = remembered {
        return FanOutProvider {
            provider,
            fan_out: Arc::clone(&runtime.fan_out),
        };
    }
    if let Some(registered) = runtime.registered.registered_v2_logger() {
        let logger = V2Logger::from_callback(&registered);
        let excluded = excluded_db_systems(runtime, &logger.config);
        attach_fan_out(runtime, from_ref(&logger.config), excluded);
        return FanOutProvider {
            provider: logger.provider,
            fan_out: Arc::clone(&runtime.fan_out),
        };
    }
    FanOutProvider {
        provider: runtime.sdk_provider.clone(),
        fan_out: Arc::clone(&runtime.fan_out),
    }
}

// attach_tenant_fan_out answers false on a repeat, which is the registry's idempotence, not an error
fn attach_fan_out(runtime: &LoggerRuntime, configs: &[Settings], excluded: BTreeSet<String>) {
    let mut registry = match runtime.fan_out.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let _attached = attach_tenant_fan_out(
        &mut registry,
        Arc::clone(&runtime.factories),
        Arc::clone(&runtime.scope),
        configs,
        excluded,
    );
}
