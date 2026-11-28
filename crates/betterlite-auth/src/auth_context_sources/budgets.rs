use crate::auth_request_models::RequestedModels;

// _get_model_names_for_budget_checks: a list keeps its order and duplicates, a string wraps once, None is [].
#[must_use]
pub fn model_names_for_budget_checks(model: &RequestedModels) -> Vec<String> {
    match model {
        RequestedModels::None => Vec::new(),
        RequestedModels::One(name) => vec![name.clone()],
        RequestedModels::Many(names) => names.clone(),
    }
}
