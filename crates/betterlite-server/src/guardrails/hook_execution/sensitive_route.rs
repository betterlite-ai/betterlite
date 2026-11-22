use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::execute::HookExecution;
use crate::guardrails::callback_caps::errors::SensitiveRouteError;

// the sensitive_data_routing proxy hook, which keeps a session on the model it was rerouted to
#[async_trait]
pub trait SensitiveRouting: Send + Sync {
    async fn set_session_routing(
        &self,
        session_id: &str,
        model: &str,
        user_api_key_dict: Option<&UserAPIKeyAuth>,
        guardrail_name: Option<&str>,
    );
}

pub async fn handle_sensitive_data_route(
    execution: &HookExecution,
    route: &SensitiveRouteError,
    data: &mut JsonMap,
    user_api_key_dict: Option<&UserAPIKeyAuth>,
) {
    let guardrail_name = route.guardrail_name.as_deref();
    tracing::info!(
        "SensitiveDataRouteException caught: session_id={} route_to_model={} guardrail={} sticky={}",
        route.session_id,
        route.route_to_model,
        guardrail_name.unwrap_or("None"),
        route.sticky_session_routing
    );
    if route.sticky_session_routing {
        if let Some(routing) = &execution.sensitive_routing {
            routing
                .set_session_routing(
                    &route.session_id,
                    &route.route_to_model,
                    user_api_key_dict,
                    guardrail_name,
                )
                .await;
        } else {
            tracing::warn!(
                "SensitiveDataRouteException requested sticky routing for session_id={} but the 'sensitive_data_routing' hook is not registered. Only this request will be rerouted; subsequent requests will not be sticky.",
                route.session_id
            );
        }
    }
    reroute(route, data);
}

fn reroute(route: &SensitiveRouteError, data: &mut JsonMap) {
    let guardrail_name = route.guardrail_name.as_deref();
    let original_model = data.get("model").cloned().unwrap_or_default();
    data.insert(
        "model".to_owned(),
        JsonValue::String(route.route_to_model.clone()),
    );
    let mut metadata = if let Some(JsonValue::Object(fields)) = data.remove("metadata") {
        fields
    } else {
        JsonMap::new()
    };
    let guardrail =
        guardrail_name.map_or(JsonValue::Null, |name| JsonValue::String(name.to_owned()));
    let detection_info = route
        .detection_info
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    metadata.insert(
        "sensitive_data_routing_applied".to_owned(),
        JsonValue::Bool(true),
    );
    metadata.insert(
        "sensitive_data_routing_original_model".to_owned(),
        original_model,
    );
    metadata.insert("sensitive_data_routing_guardrail".to_owned(), guardrail);
    metadata.insert(
        "sensitive_data_routing_detection_info".to_owned(),
        JsonValue::Object(detection_info),
    );
    data.insert("metadata".to_owned(), JsonValue::Object(metadata));
}
