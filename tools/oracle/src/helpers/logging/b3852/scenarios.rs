use std::collections::BTreeMap;
use std::slice::from_ref;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::Input;
use super::exporters::Captured;
use super::harness::HarnessError;
use super::{fixtures, harness, spans};

// the boundary and the close callback of one inference call: a recordable parent opens the routed span at pre_call, else the close defers
pub(super) fn pre_call(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let harness = harness::make_harness(
        settings,
        input.callback_name.clone(),
        None,
        harness::trace(input.anchored),
    )?;
    fixtures::boundary(
        &harness.logger,
        input.credentials.as_ref(),
        input.auth_metadata.as_ref(),
    )?;
    let builds_after_pre_call = harness.built.builds();
    let metadata = fixtures::payload_metadata(input.auth_metadata.as_ref(), None);
    let payload = fixtures::payload_document("completion", "success", metadata)?;
    let dynamic = input
        .credentials
        .as_ref()
        .map_or(Json::Null, fixtures::text_object);
    fixtures::deliver(&harness.logger, fixtures::event(payload, dynamic), false)?;
    let routed = spans::span_records(&harness.built.sinks())?;
    let default = spans::span_records(from_ref(&harness.default))?;
    Ok(json!({
        "builds_after_pre_call": builds_after_pre_call,
        "builds_after_close": harness.built.builds(),
        "routed_spans": routed,
        "default_spans": default,
    }))
}

// an MCP tool call records its own span on the default backend: it carries no LLM credentials, so nothing routes
pub(super) fn mcp_tool_call(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let harness = harness::make_harness(
        settings,
        input.callback_name.clone(),
        None,
        harness::trace(false),
    )?;
    let metadata = fixtures::payload_metadata(input.auth_metadata.as_ref(), Some("search"));
    let payload = fixtures::payload_document("call_mcp_tool", "success", metadata)?;
    fixtures::deliver(
        &harness.logger,
        fixtures::event(
            payload,
            input
                .credentials
                .as_ref()
                .map_or(Json::Null, fixtures::text_object),
        ),
        false,
    )?;
    let names = spans::span_names(from_ref(&harness.default))?;
    let count = names.len();
    Ok(json!({
        "names": names,
        "count": count,
        "builds": harness.built.builds(),
    }))
}

// a non-mapping otel.attributes filter surfaces once at ERROR, then stays silent on the next call
pub(super) fn metric_error(input: &Input) -> HelperResult {
    let mut settings = harness::settings(&input.exporters)?;
    settings.enable_metrics = true;
    let (recorder, meter_provider) = harness::metric_recorder(input.filter.clone())?;
    let harness = harness::make_harness(
        settings,
        input.callback_name.clone(),
        Some(recorder),
        harness::trace(false),
    )?;
    let metadata = fixtures::payload_metadata(None, None);
    let payload = fixtures::payload_document("completion", "success", metadata)?;
    let event = fixtures::event(payload, Json::Null);
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let outcome: Result<(), Box<HelperError>> = with_default(subscriber, || {
        fixtures::deliver(&harness.logger, Arc::clone(&event), false)?;
        fixtures::deliver(&harness.logger, event, false)
    });
    outcome?;
    drop(meter_provider);
    let error_logs = captured.0.load(Ordering::Relaxed);
    Ok(json!({"error_logs": error_logs}))
}

// the first close pops the carrier and emits; the second close with the same call id finds none and the emitter's marker drops it
pub(super) fn close_dedup(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let harness = harness::make_harness(
        settings,
        input.callback_name.clone(),
        None,
        harness::trace(true),
    )?;
    fixtures::boundary(&harness.logger, None, None)?;
    let metadata = fixtures::payload_metadata(None, None);
    let success = fixtures::payload_document("completion", "success", metadata.clone())?;
    fixtures::deliver(&harness.logger, fixtures::event(success, Json::Null), false)?;
    let failure = fixtures::payload_document("completion", "failure", metadata)?;
    fixtures::deliver(&harness.logger, fixtures::event(failure, Json::Null), true)?;
    let names = spans::span_names(from_ref(&harness.default))?;
    let count = names.len();
    Ok(json!({"names": names, "count": count}))
}

// the LRU bound holds 256 providers: the 257th space evicts the first, which drains its held lease, and routing the last space again hits
pub(super) fn cache_eviction(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let cache = harness::make_cache(settings, input.callback_name.clone())?;
    let mut held = None;
    for (index, space) in input.spaces.iter().enumerate() {
        let params = fixtures::space_params(space);
        let route = cache
            .cache
            .route_for(&cache.default, Some(&params), None, &[])
            .or_config_error()?;
        if index == 0 {
            held.clone_from(&route.provider);
        } else {
            cache.cache.release(route.provider.as_ref());
        }
    }
    let builds = cache.built.builds();
    cache.cache.release(held.as_ref());
    let shutdowns = spans::shutdowns(&cache.built.sinks());
    let last = input
        .spaces
        .last()
        .ok_or(HarnessError::NoSpaces)
        .or_value_error()?;
    let params = fixtures::space_params(last);
    cache
        .cache
        .route_for(&cache.default, Some(&params), None, &[])
        .or_config_error()?;
    let builds_after_reuse = cache.built.builds();
    Ok(json!({
        "builds": builds,
        "shutdowns_after_release": shutdowns,
        "builds_after_reuse": builds_after_reuse,
    }))
}

// the routed exporter the tenant cache builds: credentials replace the static headers, and a key/team project appends a percent-encoded header
pub(super) fn project_headers(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let cache = harness::make_cache(settings, input.callback_name.clone())?;
    let auth_metadata: Option<BTreeMap<String, String>> = input
        .project_name
        .as_ref()
        .map(|name| BTreeMap::from([("phoenix_project_name".to_owned(), name.clone())]));
    let params = input.credentials.as_ref().map(fixtures::text_object);
    let route = cache
        .cache
        .route_for(&cache.default, params.as_ref(), auth_metadata.as_ref(), &[])
        .or_config_error()?;
    let spec = cache
        .built
        .specs()
        .into_iter()
        .next()
        .ok_or(HarnessError::NoProvider)
        .or_value_error()?;
    let mut header_pairs: Vec<String> = spec
        .headers
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::to_owned)
        .collect();
    header_pairs.sort();
    let builds = cache.built.builds();
    Ok(json!({
        "builds": builds,
        "header_pairs": header_pairs,
        "endpoint": spec.endpoint,
        "detached": route.detached,
    }))
}
