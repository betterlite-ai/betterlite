use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use indexmap::IndexMap;

use crate::otel::integrations::arize_settings::ArizeConfig;
use crate::otel::legacy_lifecycle::error::LegacyLifecycleError;
use crate::otel::legacy_lifecycle::lifecycle::{
    ExportGate, LegacyLifecycle, LegacyLifecycleOptions, LifecycleEvent,
};
use crate::otel::legacy_lifecycle::state::request_key;
use crate::otel::legacy_lifecycle::tracer::{DynamicOtel, dynamic_params};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::presets::arize::{arize_dynamic_headers, arize_headers};
use crate::otel_provider_assembly::legacy_otel_init::LegacyOtel;

mod tests;

pub const CALLBACK_NAME: &str = "arize";
pub const SUCCESS_RATE_VAR: &str = "arize_success_sampling_rate";
pub const ERROR_RATE_VAR: &str = "arize_error_sampling_rate";
pub const HEALTHY_MESSAGE: &str = "Arize credentials are configured properly";
pub const MISSING_SPACE_MESSAGE: &str =
    "ARIZE_SPACE_ID or ARIZE_SPACE_KEY environment variable not set";
pub const MISSING_API_KEY_MESSAGE: &str = "ARIZE_API_KEY environment variable not set";

const MAX_TRACKED_REQUESTS: usize = 10_000;
const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

type Draw = Box<dyn Fn() -> f64 + Send + Sync>;

// ArizeLogger._should_export: the decision is cached on the request, so success and failure hooks agree
pub struct ArizeSampling {
    draw: Draw,
    decisions: Mutex<IndexMap<String, bool>>,
}

impl Default for ArizeSampling {
    fn default() -> Self {
        Self::new(system_draw)
    }
}

// random.random() is a uniform draw in [0, 1); the subsecond clock has the same shape and needs no global seed
fn system_draw() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    f64::from(nanos).mul_add(NANOS_PER_SECOND.recip(), 0.0)
}

impl ArizeSampling {
    #[must_use]
    pub fn new<D>(draw: D) -> Self
    where
        D: Fn() -> f64 + Send + Sync + 'static,
    {
        Self {
            draw: Box::new(draw),
            decisions: Mutex::new(IndexMap::new()),
        }
    }

    fn sampled(
        &self,
        kwargs: &MetadataMap,
        var: &'static str,
    ) -> Result<bool, LegacyLifecycleError> {
        let Some(rate) = sampling_rate(kwargs, var)? else {
            return Ok(true);
        };
        let key = request_key(kwargs).map(|request| format!("arize_sampled:{request}:{var}"));
        let mut decisions = self
            .decisions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(key) = &key
            && let Some(cached) = decisions.get(key)
        {
            return Ok(*cached);
        }
        let sampled = rate > 0.0 && (self.draw)() <= rate;
        if let Some(key) = key {
            if decisions.len() >= MAX_TRACKED_REQUESTS {
                _ = decisions.shift_remove_index(0);
            }
            decisions.insert(key, sampled);
        }
        Ok(sampled)
    }
}

impl ExportGate for ArizeSampling {
    fn should_export(
        &self,
        kwargs: &MetadataMap,
        event: LifecycleEvent,
    ) -> Result<bool, LegacyLifecycleError> {
        let var = match event {
            LifecycleEvent::Success => SUCCESS_RATE_VAR,
            LifecycleEvent::Failure => ERROR_RATE_VAR,
        };
        self.sampled(kwargs, var)
    }
}

// _sampling_rate_for_request: absent, null, non-finite and out-of-range rates leave the request exported
fn sampling_rate(
    kwargs: &MetadataMap,
    var: &'static str,
) -> Result<Option<f64>, LegacyLifecycleError> {
    let Some(params) = dynamic_params(kwargs)? else {
        return Ok(None);
    };
    match params.get(var) {
        None | Some(OtelModelValue::Null(())) => Ok(None),
        Some(OtelModelValue::Number(number)) => Ok(number
            .as_f64()
            .filter(|rate| rate.is_finite() && (0.0..=1.0).contains(rate))),
        Some(
            OtelModelValue::Bool(_)
            | OtelModelValue::String(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_),
        ) => Err(LegacyLifecycleError::SamplingRate { field: var }),
    }
}

pub struct ArizeDynamicOtel;

impl DynamicOtel for ArizeDynamicOtel {
    fn construct_dynamic_otel_headers(
        &self,
        dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        let headers = arize_dynamic_headers(&dynamic_strings(dynamic_params));
        (!headers.is_empty()).then_some(headers)
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        None
    }
}

// the three params ArizeLogger.construct_dynamic_otel_headers reads out of the dynamic bucket
fn dynamic_strings(dynamic_params: &MetadataMap) -> BTreeMap<String, String> {
    ["arize_space_id", "arize_space_key", "arize_api_key"]
        .iter()
        .filter_map(|name| {
            let value = dynamic_params.get(*name).and_then(OtelModelValue::as_str)?;
            Some(((*name).to_owned(), value.to_owned()))
        })
        .collect()
}

// the registration config litellm_logging builds for the arize integration
#[must_use]
pub fn otel_config(environment: &BTreeMap<String, String>) -> OpenTelemetryConfig {
    let arize = ArizeConfig::from_env(environment);
    let mut config = OpenTelemetryConfig::from_env(environment);
    config.exporter = Exporter::Named(arize.protocol.exporter_kind().to_owned());
    config.endpoint = Some(arize.endpoint.clone());
    config.headers = arize_headers(environment, &arize);
    if let Some(project) = arize.project_name {
        config.service_name = project;
        // __post_init__ then reads OTEL_MODEL_ID before falling back to the project name
        config.model_id = String::new();
    }
    config.normalize(environment);
    config
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ArizeHealth {
    Healthy { message: &'static str },
    Unhealthy { error_message: String },
}

#[must_use]
pub fn health_check(environment: &dyn EnvironmentLookup) -> ArizeHealth {
    let config = ArizeConfig::from_env(environment);
    if config.space_id.is_none() && config.space_key.is_none() {
        return ArizeHealth::Unhealthy {
            error_message: MISSING_SPACE_MESSAGE.to_owned(),
        };
    }
    if config.api_key.is_none() {
        return ArizeHealth::Unhealthy {
            error_message: MISSING_API_KEY_MESSAGE.to_owned(),
        };
    }
    ArizeHealth::Healthy {
        message: HEALTHY_MESSAGE,
    }
}

#[must_use]
pub fn arize_lifecycle(otel: LegacyOtel, mut options: LegacyLifecycleOptions) -> LegacyLifecycle {
    options.dynamic = Arc::new(ArizeDynamicOtel);
    let mut lifecycle = LegacyLifecycle::from_otel(otel, options);
    lifecycle.export_gate = Some(Arc::new(ArizeSampling::default()));
    lifecycle
}
