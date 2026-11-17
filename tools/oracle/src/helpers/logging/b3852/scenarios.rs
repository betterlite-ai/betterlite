use std::collections::BTreeMap;
use std::slice::from_ref;
use std::sync::Arc;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;
use tracing_subscriber::layer::SubscriberExt as _;

use super::Input;
use super::harness;

// the boundary and the close callback of one inference call: a recordable parent opens the routed
// span at pre_call, while an unanchored request defers every build to the close
pub(super) fn pre_call(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let harness = harness::harness(
        settings,
        input.callback_name.clone(),
        None,
        harness::trace(input.anchored),
    )?;
    harness::boundary(
        &harness.logger,
        input.credentials.as_ref(),
        input.auth_metadata.as_ref(),
    )?;
    let builds_after_pre_call = harness.built.builds();
    let metadata = harness::payload_metadata(input.auth_metadata.as_ref(), None);
    let payload = harness::payload_document("completion", "success", metadata)?;
    let dynamic = harness::dynamic_params(input.credentials.as_ref());
    harness::deliver(&harness.logger, harness::event(payload, dynamic), false)?;
    let routed = harness::span_records(&harness.built.sinks())?;
    let default = harness::span_records(from_ref(&harness.default))?;
    Ok(json!({
        "builds_after_pre_call": builds_after_pre_call,
        "builds_after_close": harness.built.builds(),
        "routed_spans": routed,
        "default_spans": default,
    }))
}

// an MCP tool call records its own span on the default backend: it carries no LLM credentials, so
// nothing routes to a tenant provider
pub(super) fn mcp_tool_call(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let harness = harness::harness(
        settings,
        input.callback_name.clone(),
        None,
        harness::trace(false),
    )?;
    let metadata = harness::payload_metadata(input.auth_metadata.as_ref(), Some("search"));
    let payload = harness::payload_document("call_mcp_tool", "success", metadata)?;
    harness::deliver(
        &harness.logger,
        harness::event(payload, harness::dynamic_params(input.credentials.as_ref())),
        false,
    )?;
    let names = harness::span_names(from_ref(&harness.default))?;
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
    let harness = harness::harness(
        settings,
        input.callback_name.clone(),
        Some(recorder),
        harness::trace(false),
    )?;
    let metadata = harness::payload_metadata(None, None);
    let payload = harness::payload_document("completion", "success", metadata)?;
    let event = harness::event(payload, harness::dynamic_params(None));
    let captured = harness::Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let outcome: Result<(), Box<HelperError>> =
        tracing::subscriber::with_default(subscriber, || {
            harness::deliver(&harness.logger, Arc::clone(&event), false)?;
            harness::deliver(&harness.logger, event, false)
        });
    outcome?;
    drop(meter_provider);
    let error_logs = captured.count();
    Ok(json!({"error_logs": error_logs}))
}

// the first close pops the carrier and emits; the second close with the same call id finds none and
// is dropped by the emitter's (call id, role) marker
pub(super) fn close_dedup(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let harness = harness::harness(
        settings,
        input.callback_name.clone(),
        None,
        harness::trace(true),
    )?;
    harness::boundary(&harness.logger, None, None)?;
    let metadata = harness::payload_metadata(None, None);
    let success = harness::payload_document("completion", "success", metadata.clone())?;
    harness::deliver(
        &harness.logger,
        harness::event(success, harness::dynamic_params(None)),
        false,
    )?;
    let failure = harness::payload_document("completion", "failure", metadata)?;
    harness::deliver(
        &harness.logger,
        harness::event(failure, harness::dynamic_params(None)),
        true,
    )?;
    let names = harness::span_names(from_ref(&harness.default))?;
    let count = names.len();
    Ok(json!({"names": names, "count": count}))
}

// the LRU bound holds 256 providers: the 257th space evicts the first, which stays alive while its
// lease is held, is shut down when the lease goes, and routing the last space again is a cache hit
pub(super) fn cache_eviction(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let cache = harness::cache(settings, input.callback_name.clone())?;
    let mut held = None;
    for (index, space) in input.spaces.iter().enumerate() {
        let params = harness::space_params(space);
        let route = cache
            .cache
            .route_for(&cache.default, Some(&params), None, &[])
            .or_config_error()?;
        if index == 0 {
            held = route.provider.clone();
        } else {
            cache.cache.release(route.provider.as_ref());
        }
    }
    let builds = cache.built.builds();
    cache.cache.release(held.as_ref());
    let shutdowns = harness::shutdowns(&cache.built.sinks());
    let last = input
        .spaces
        .last()
        .ok_or(harness::HarnessError::NoSpaces)
        .or_value_error()?;
    let params = harness::space_params(last);
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

// the routed exporter the tenant cache builds: credentials replace the static headers, a key/team
// project appends its own percent-encoded header, and both detach the span from the request trace
pub(super) fn project_headers(input: &Input) -> HelperResult {
    let settings = harness::settings(&input.exporters)?;
    let cache = harness::cache(settings, input.callback_name.clone())?;
    let auth_metadata: Option<BTreeMap<String, String>> = input
        .project_name
        .as_ref()
        .map(|name| BTreeMap::from([("phoenix_project_name".to_owned(), name.clone())]));
    let params = input.credentials.as_ref().map(harness::text_object);
    let route = cache
        .cache
        .route_for(&cache.default, params.as_ref(), auth_metadata.as_ref(), &[])
        .or_config_error()?;
    let spec = cache
        .built
        .specs()
        .into_iter()
        .next()
        .ok_or(harness::HarnessError::NoProvider)
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
