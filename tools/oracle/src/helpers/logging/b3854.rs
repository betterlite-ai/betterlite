use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_policy::capture::LoggingFlags;
use betterlite_logging::otel::legacy_policy::error_recording::RecordedException;
use betterlite_logging::otel::legacy_policy::semconv_attrs::semconv_latest_experimental;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;

use crate::helpers::HelperResult;

use self::config_input::ConfigInput;
use self::inputs::{EmitCall, IdentityInput, MetadataInput, SpanNameInput, TeamInput, TimeInput};

mod arms;
mod config_input;
mod inputs;
mod render;
mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ConfigFromEnv {
        environment: BTreeMap<String, String>,
    },
    ConfigNormalize {
        environment: BTreeMap<String, String>,
        config: ConfigInput,
    },
    MetadataKeys {
        values: Vec<MetadataInput>,
    },
    CaptureSample {
        configs: Vec<ConfigInput>,
        environment: BTreeMap<String, String>,
    },
    CaptureResolve {
        config: ConfigInput,
        flags: LoggingFlags,
    },
    EmitOnce {
        calls: Vec<EmitCall>,
    },
    SpanTime {
        times: Vec<TimeInput>,
        now: i64,
        #[serde(default)]
        names: Vec<SpanNameInput>,
    },
    Team {
        teams: Vec<TeamInput>,
    },
    TeamFromPayload {
        payload: BTreeMap<String, OtelModelValue>,
    },
    Identity(IdentityInput),
    ServiceTier {
        payloads: Vec<BTreeMap<String, OtelModelValue>>,
    },
    ExceptionStamp {
        exception: Option<RecordedException>,
        payload: BTreeMap<String, OtelModelValue>,
    },
    SemconvRequest {
        optional_params: BTreeMap<String, OtelModelValue>,
    },
    SemconvCacheAndOperation {
        payloads: Vec<BTreeMap<String, OtelModelValue>>,
        #[serde(default)]
        call_types: Vec<Option<String>>,
    },
    SemconvGate {
        config: ConfigInput,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ConfigFromEnv { environment } => Ok(arms::config_from_env(environment)),
            Input::ConfigNormalize {
                environment,
                config,
            } => Ok(arms::config_normalize(config, environment)),
            Input::MetadataKeys { values } => Ok(arms::metadata_keys(values)),
            Input::CaptureSample {
                configs,
                environment,
            } => Ok(arms::capture_sample(configs, environment)),
            Input::CaptureResolve { config, flags } => Ok(arms::capture_resolve(config, *flags)),
            Input::EmitOnce { calls } => Ok(arms::emit_once(calls)),
            Input::SpanTime { times, now, names } => Ok(arms::span_time(times, *now, names)),
            Input::Team { teams } => arms::team_attributes(teams),
            Input::TeamFromPayload { payload } => arms::team_from_payload(payload),
            Input::Identity(identity) => arms::identity(identity),
            Input::ServiceTier { payloads } => arms::service_tier(payloads),
            Input::ExceptionStamp { exception, payload } => {
                arms::exception_stamp(exception.as_ref(), payload)
            }
            Input::SemconvRequest { optional_params } => arms::semconv_request(optional_params),
            Input::SemconvCacheAndOperation {
                payloads,
                call_types,
            } => arms::semconv_cache_and_operation(payloads, call_types),
            Input::SemconvGate { config } => Ok(serde_json::json!(semconv_latest_experimental(
                &config.to_config()
            ))),
        }
    }
}
