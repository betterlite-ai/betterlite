use crate::auth_request_models::RequestedModels;

// Python's `if not model`: an empty string and an empty list are falsey, a list holding a name never is
#[must_use]
pub(crate) fn is_falsey(model: &RequestedModels) -> bool {
    match model {
        RequestedModels::None => true,
        RequestedModels::One(name) => name.is_empty(),
        RequestedModels::Many(names) => names.is_empty(),
    }
}

// the model as the denial messages spell it, matching can_object_call_model's depth message
#[must_use]
pub(crate) fn requested_text(model: &RequestedModels) -> String {
    match model {
        RequestedModels::None => String::new(),
        RequestedModels::One(name) => name.clone(),
        RequestedModels::Many(names) => names.join(","),
    }
}
