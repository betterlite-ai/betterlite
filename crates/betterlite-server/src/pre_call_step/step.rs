use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_auth::auth_route_patterns::canonical::{RequestRouteSource, get_request_route};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_destination::parameters::tags_from_request_body;
use betterlite_core::api::request_metadata_safety::metadata;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::datadog::apm::NullTracer;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, Utc};
use http::request::Parts;

use super::call_logging::{build_logging, identify, stream_usage};
use super::guardrail_phase::{attach, check_added_tags, finish, hook, merge_guardrails, prefetch};
use super::json::to_request_data;
use super::request_setup::{authorize_stores, drop_flag, prepare, rewrite_model, router_phase};
use super::standalone::{
    AllowAllBudgets, AllowAllStores, NeverSupports, NoCompression, NoLoggers, NoRetrieval,
    NoRouterSettings, PassthroughHooks, PassthroughMerge, PassthroughPreparer,
};
use super::{
    PreCallError, PreCallInput, PreCallOutput, PreCallSettings, PreCallStep, held_error, internal,
};
use crate::auth::route_match::decoded_route_path;
use crate::pre_call::failure::{PreCallFailure, ProxyFailure};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::PreCallPorts;
use crate::pre_call::rewrites::write_queue_time;

impl<K: PreCallKey> PreCallStep<K> {
    #[must_use]
    pub fn new(
        ports: PreCallPorts<K>,
        settings: PreCallSettings,
        loggers: Arc<dyn LoggerResolver>,
    ) -> Self {
        Self {
            ports,
            settings,
            loggers,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH),
        }
    }

    // the stand-in every B1100 route module builds until B8804 hands it the production ports
    #[must_use]
    pub fn standalone(settings: PreCallSettings) -> Self {
        let ports: PreCallPorts<K> = PreCallPorts {
            preparer: Arc::new(PassthroughPreparer),
            vector_stores: Arc::new(AllowAllStores),
            router_settings: Arc::new(NoRouterSettings),
            router: None,
            stream_options: Arc::new(NeverSupports),
            guardrail_merge: Arc::new(PassthroughMerge),
            compression: Arc::new(NoCompression),
            hooks: Arc::new(PassthroughHooks),
            tag_budgets: Arc::new(AllowAllBudgets),
            retrieval: Arc::new(NoRetrieval),
            tracer: Arc::new(NullTracer),
        };
        Self::new(ports, settings, Arc::new(NoLoggers))
    }

    pub async fn run(
        &self,
        data: JsonMap,
        input: &PreCallInput<'_, K>,
        rate_limited_model: Option<&str>,
    ) -> Result<PreCallOutput, PreCallError> {
        let mut tags_before = None;
        self.run_tracked(data, input, rate_limited_model, &mut tags_before)
            .await
    }

    // the fallback retry passes the first attempt's tag set in, since LiteLLM keeps it on the processor
    pub(super) async fn run_tracked(
        &self,
        data: JsonMap,
        input: &PreCallInput<'_, K>,
        rate_limited_model: Option<&str>,
        tags_before: &mut Option<BTreeSet<String>>,
    ) -> Result<PreCallOutput, PreCallError> {
        let started = SystemTime::now();
        let (mut final_data, mut logging) = self.front_half(data, input, started).await?;
        final_data = merge_guardrails(self, final_data, rate_limited_model).await;
        self.ports.compression.arm(&mut final_data).await;
        record_tags(&final_data, tags_before);
        prefetch(self, &final_data);
        final_data = hook(self, final_data, input).await?;
        check_added_tags(self, &final_data, input, tags_before).await?;
        attach(self, &mut final_data, input).await;
        finish(self, &mut final_data, &mut logging)?;
        Ok(PreCallOutput {
            data: final_data,
            logging,
        })
    }

    async fn front_half(
        &self,
        mut data: JsonMap,
        input: &PreCallInput<'_, K>,
        started: SystemTime,
    ) -> Result<(JsonMap, LoggingState), PreCallError> {
        if let Some(failure) = reject_model(&data) {
            return Err(PreCallError { failure, data });
        }
        data = prepare(self.ports.preparer.as_ref(), data, input).await?;
        drop_flag(self, &mut data)?;
        authorize_stores(self, &data, input).await?;
        let route = request_route(input.request);
        // ast-grep-ignore: no-long-names
        let bucket = metadata::metadata_variable_name_for_route(&route);
        let queue_start = held_error(data.clone(), started_seconds(started))?;
        write_queue_time(&mut data, bucket.as_str(), queue_start);
        rewrite_model(&self.settings, &mut data, input)?;
        router_phase(self, &mut data, input).await?;
        let call_id = identify(self, &mut data, input);
        stream_usage(self, &mut data, input.route_type, input.key.team_id());
        let wall_clock = DateTime::<Utc>::from(started).fixed_offset();
        let logging = build_logging(self, &mut data, input, &call_id, wall_clock)?;
        Ok((data, logging))
    }
}

fn reject_model(data: &JsonMap) -> Option<PreCallFailure> {
    let requested = data.get("model")?;
    match requested {
        JsonValue::Null | JsonValue::String(_) => None,
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Some(PreCallFailure::Proxy(ProxyFailure {
                message: String::from("'model' must be a string."),
                error_type: ProxyErrorTypes::BadRequestError.to_string(),
                param: Some(String::from("model")),
                code: 400,
            }))
        }
    }
}

fn record_tags(data: &JsonMap, tags_before: &mut Option<BTreeSet<String>>) {
    if tags_before.is_none() {
        *tags_before = Some(
            tags_from_request_body(&to_request_data(data))
                .into_iter()
                .collect(),
        );
    }
}

pub(super) fn request_route(request: &Parts) -> String {
    let url_path = request.uri.path();
    // an Axum URI is already the dispatched path, so no ASGI scope root applies
    let path = decoded_route_path(url_path).unwrap_or_else(|| url_path.to_owned());
    get_request_route(&RequestRouteSource {
        scope: Ok(None),
        url_path: path.as_str(),
    })
}

fn started_seconds(started: SystemTime) -> Result<f64, PreCallFailure> {
    let elapsed = started.duration_since(UNIX_EPOCH).map_err(|_error| {
        internal(String::from(
            "the system clock reads a time before the Unix epoch",
        ))
    })?;
    Ok(elapsed.as_secs_f64())
}
