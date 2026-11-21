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
use super::{PreCallError, PreCallInput, PreCallStep};
use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::rewrites::{apply_aliases, apply_overrides, file_search_store_ids};
use crate::pre_call::route_type::RouteType;
use crate::stream_cleanup::fallback_flag::drop_fallback_errors_flag;

pub(super) async fn prepare<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<JsonMap, PreCallError> {
    let arrival = data.clone();
    step.ports
        .preparer
        .prepare(data, input.request, input.key, input.version)
        .await
        .map_err(|failure| PreCallError {
            failure,
            data: arrival,
        })
}

pub(super) fn drop_flag<K: PreCallKey>(step: &PreCallStep<K>, data: &mut JsonMap) {
    let mut request_body = to_request_data(data);
    drop_fallback_errors_flag(&step.settings.fallback_errors, &mut request_body);
    *data = from_request_data(request_body);
}

pub(super) async fn authorize_stores<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<(), PreCallError> {
    if !matches!(
        input.route_type,
        RouteType::Responses | RouteType::ResponsesWebsocket
    ) {
        return Ok(());
    }
    let store_ids = file_search_store_ids(data).map_err(|failure| PreCallError {
        failure,
        data: data.clone(),
    })?;
    for store_id in store_ids {
        step.ports
            .vector_stores
            .assert_access(&store_id, input.key)
            .await
            .map_err(|failure| PreCallError {
                failure,
                data: data.clone(),
            })?;
    }
    Ok(())
}

pub(super) fn rewrite_model<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<(), PreCallError> {
    if let Some(endpoint_model) = input.model {
        reject_url_valued_destination(DestinationField::Model, endpoint_model, &[]).map_err(
            |error| PreCallError {
                failure: destination_failure(&error),
                data: data.clone(),
            },
        )?;
    }
    let kind = if input.route_type == RouteType::ImageEdit {
        InferenceModelKind::ImageEdit
    } else {
        InferenceModelKind::Completion
    };
    let resolved = resolve_inference_model(
        data.get("model").and_then(JsonValue::as_str),
        &step.settings.model_defaults,
        step.settings.user_model.as_deref(),
        input.model,
        kind,
    );
    data.insert(
        String::from("model"),
        resolved.map_or(JsonValue::Null, JsonValue::String),
    );
    apply_overrides(data, &step.settings.overrides);
    apply_aliases(data, &step.settings.pre_call.model_alias_map, input.key);
    Ok(())
}

pub(super) async fn router_phase<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) -> Result<(), PreCallError> {
    let Some(router) = step.ports.router.as_deref() else {
        return Ok(());
    };
    let settings = step
        .ports
        .router_settings
        .router_settings(input.key)
        .await
        .map_err(|failure| PreCallError {
            failure,
            data: data.clone(),
        })?;
    let Some(settings) = settings else {
        return Ok(());
    };
    let held = data.clone();
    apply_router_settings(data, &settings, router, input.key)
        .await
        .map_err(|failure| PreCallError {
            failure,
            data: held,
        })
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
