use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_destination::destination::{
    DestinationField, reject_url_valued_destination,
};
use betterlite_core::api::request_destination::error::RequestDestinationError;
use betterlite_core::api::request_destination::model::{
    InferenceModelKind, resolve_inference_model,
};
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::json::{from_request_data, to_request_data};
use super::router_branch::apply_router_settings;
use super::{PreCallError, PreCallInput, PreCallSettings, PreCallStep, held_error};
use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::RequestPreparer;
use crate::pre_call::rewrites::{apply_aliases, apply_overrides, file_search_store_ids};
use crate::pre_call::route_type::RouteType;
use crate::stream_cleanup::fallback_flag::drop_fallback_errors_flag;

pub(super) async fn prepare<K>(
    preparer: &dyn RequestPreparer<K>,
    data: JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<JsonMap, PreCallError>
where
    K: PreCallKey,
{
    let arrival = data.clone();
    let answer = preparer
        .prepare(data, input.request, input.key, input.version)
        .await;
    held_error(arrival, answer)
}

pub(super) fn drop_flag<K>(step: &PreCallStep<K>, data: &mut JsonMap) -> Result<(), PreCallError>
where
    K: PreCallKey,
{
    let mut request_body = to_request_data(data);
    drop_fallback_errors_flag(&step.settings.fallback_errors, &mut request_body);
    let held = data.clone();
    *data = held_error(held, from_request_data(request_body))?;
    Ok(())
}

pub(super) async fn authorize_stores<K>(
    step: &PreCallStep<K>,
    data: &JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<(), PreCallError>
where
    K: PreCallKey,
{
    if !matches!(
        input.route_type,
        RouteType::Responses | RouteType::ResponsesWebsocket
    ) {
        return Ok(());
    }
    let store_ids = held_error(data.clone(), file_search_store_ids(data))?;
    for store_id in store_ids {
        let granted = step
            .ports
            .vector_stores
            .assert_access(&store_id, input.key)
            .await;
        held_error(data.clone(), granted)?;
    }
    Ok(())
}

pub(super) fn rewrite_model<K>(
    settings: &PreCallSettings,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<(), PreCallError>
where
    K: PreCallKey,
{
    if let Some(endpoint_model) = input.model {
        let checked = reject_url_valued_destination(DestinationField::Model, endpoint_model, &[])
            .map_err(|error| destination_failure(&error));
        held_error(data.clone(), checked)?;
    }
    let kind = if input.route_type == RouteType::ImageEdit {
        InferenceModelKind::ImageEdit
    } else {
        InferenceModelKind::Completion
    };
    let resolved = resolve_inference_model(
        data.get("model").and_then(JsonValue::as_str),
        &settings.model_defaults,
        settings.user_model.as_deref(),
        input.model,
        kind,
    );
    data.insert(
        String::from("model"),
        resolved.map_or(JsonValue::Null, JsonValue::String),
    );
    apply_overrides(data, &settings.overrides);
    apply_aliases(data, &settings.pre_call.model_alias_map, input.key);
    Ok(())
}

pub(super) async fn router_phase<K>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<(), PreCallError>
where
    K: PreCallKey,
{
    let Some(router) = step.ports.router.as_deref() else {
        return Ok(());
    };
    let resolved = step.ports.router_settings.router_settings(input.key).await;
    let Some(settings) = held_error(data.clone(), resolved)? else {
        return Ok(());
    };
    let held = data.clone();
    let applied = apply_router_settings(data, &settings, router, input.key).await;
    held_error(held, applied)
}

fn destination_failure(error: &RequestDestinationError) -> PreCallFailure {
    let facts = error.facts();
    let mut detail = JsonMap::new();
    if let Some(entered) = facts.detail {
        detail.insert(
            String::from("error"),
            JsonValue::String(entered.error.to_owned()),
        );
        detail.insert(
            String::from("param"),
            JsonValue::String(entered.param.to_string()),
        );
    }
    detail.insert(
        String::from("message"),
        JsonValue::String(error.to_string()),
    );
    PreCallFailure::Http {
        status: facts.status,
        detail: JsonValue::Object(detail),
    }
}
