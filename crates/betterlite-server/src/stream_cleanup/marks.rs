use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

const CLIENT_DISCONNECTED_CODE: &str = "499";

#[must_use]
pub fn disconnect_error_info() -> JsonObject {
    let mut info = JsonObject::new();
    info.insert(
        "error_code".to_owned(),
        Json::Text(CLIENT_DISCONNECTED_CODE.to_owned()),
    );
    info.insert(
        "error_message".to_owned(),
        Json::Text("Client disconnected the request".to_owned()),
    );
    info.insert(
        "error_class".to_owned(),
        Json::Text("ClientDisconnected".to_owned()),
    );
    info
}

pub fn apply_disconnect_marks(target: &mut JsonObject) {
    target.insert("client_disconnected".to_owned(), Json::Bool(true));
    target.insert(
        "error_information".to_owned(),
        Json::Object(disconnect_error_info()),
    );
}

pub fn mark_request_disconnected(request_data: &mut JsonObject) {
    mark_nested(request_data, "metadata");
    if let Some(params) = metadata_object(request_data, "litellm_params") {
        mark_nested(params, "metadata");
    }
}

pub fn mark_details_disconnected(details: &mut JsonObject) {
    if let Some(params) = metadata_object(details, "litellm_params") {
        mark_nested(params, "metadata");
    }
    mark_nested(details, "metadata");
}

fn mark_nested(container: &mut JsonObject, key: &str) {
    if let Some(metadata) = metadata_object(container, key) {
        apply_disconnect_marks(metadata);
    }
}

fn metadata_object<'container>(
    container: &'container mut JsonObject,
    key: &str,
) -> Option<&'container mut JsonObject> {
    if matches!(container.get(key), None | Some(Json::Null)) {
        container.insert(key.to_owned(), Json::Object(JsonObject::new()));
    }
    match container.get_mut(key) {
        Some(Json::Object(nested)) => Some(nested),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}
