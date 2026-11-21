use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_destination::parameters::tags_from_request_body;
use betterlite_core::api::request_metadata_safety::snapshot;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::update_messages;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::json::{from_request_data, to_decision, to_request_data};
use super::step::request_route;
use super::{PreCallError, PreCallInput, PreCallStep, held_error};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::rewrites::request_model;
use crate::pre_call::route_type::RouteType;

pub(super) async fn merge_guardrails<K>(
    step: &PreCallStep<K>,
    mut data: JsonMap,
    rate_limited_model: Option<&str>,
) -> JsonMap
where
    K: PreCallKey,
{
    if let Some(alias) = rate_limited_model {
        data = step.ports.guardrail_merge.merge(data, Some(alias)).await;
    }
    step.ports.guardrail_merge.merge(data, None).await
}

pub(super) fn prefetch<K>(step: &PreCallStep<K>, data: &JsonMap)
where
    K: PreCallKey,
{
    let Some(router) = step.ports.router.as_deref() else {
        return;
    };
    let Some(model) = data.get("model").and_then(JsonValue::as_str) else {
        return;
    };
    router.arm_prefetch(model, data);
}

pub(super) async fn hook<K>(
    step: &PreCallStep<K>,
    data: JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<JsonMap, PreCallError>
where
    K: PreCallKey,
{
    let held = data.clone();
    step.ports
        .hooks
        .pre_call_hook(input.key, data, input.route_type, input.skip_guardrails)
        .await
        .map_err(|failure| PreCallError {
            failure,
            data: held,
        })
}

pub(super) async fn check_added_tags<K>(
    step: &PreCallStep<K>,
    data: &JsonMap,
    input: &PreCallInput<'_, K>,
    tags_before: &mut Option<BTreeSet<String>>,
) -> Result<(), PreCallError>
where
    K: PreCallKey,
{
    let known = tags_before.get_or_insert_with(BTreeSet::new);
    let added: Vec<String> = tags_from_request_body(&to_request_data(data))
        .into_iter()
        .filter(|tag| !known.contains(tag))
        .collect();
    if added.is_empty() {
        return Ok(());
    }
    let route = request_route(input.request);
    let skips = step
        .ports
        .tag_budgets
        .skips_budget(&route, &request_model(data))
        .await;
    if skips {
        return Ok(());
    }
    step.ports
        .tag_budgets
        .check_tags(&added, input.key)
        .await
        .map_err(|failure| PreCallError {
            failure,
            data: data.clone(),
        })
}

pub(super) async fn attach<K>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) where
    K: PreCallKey,
{
    if input.route_type == RouteType::GetResponses {
        step.ports.retrieval.attach(data, input.key).await;
    }
}

pub(super) fn finish(data: &mut JsonMap, logging: &mut LoggingState) -> Result<(), PreCallError> {
    let mut request_body = to_request_data(data);
    // ast-grep-ignore: no-long-names
    snapshot::refresh_proxy_server_request_body_snapshot(
        &mut request_body,
        Some(&mut *logging),
        true,
    );
    let held = data.clone();
    *data = held_error(held, from_request_data(request_body))?;
    tracing::debug!("receiving data: {data:?}");
    if let Some(JsonValue::Array(messages)) = data.get("messages") {
        update_messages(logging, messages.iter().map(to_decision).collect());
    }
    Ok(())
}
