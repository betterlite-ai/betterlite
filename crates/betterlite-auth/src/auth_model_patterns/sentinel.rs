use crate::auth_model_patterns::ALL_TEAM_MODELS;
use crate::auth_model_patterns::view::ModelAccessView;

#[must_use]
pub fn resolve_team_model_sentinel(
    models: &[String],
    view: Option<&dyn ModelAccessView>,
    team_id: Option<&str>,
) -> Vec<String> {
    let carries_sentinel = models.iter().any(|model| model.as_str() == ALL_TEAM_MODELS);
    let (Some(view), Some(_)) = (view, team_id) else {
        return models.to_vec();
    };
    if !carries_sentinel {
        return models.to_vec();
    }
    let proxy_models = view.model_names();
    let without_sentinel: Vec<String> = models
        .iter()
        .filter(|model| model.as_str() != ALL_TEAM_MODELS)
        .cloned()
        .collect();
    if proxy_models.is_empty() {
        return if without_sentinel.is_empty() {
            models.to_vec()
        } else {
            without_sentinel
        };
    }
    // dict.fromkeys(non_sentinel + proxy_models): first-seen order over the whole concatenation
    let mut resolved: Vec<String> = Vec::new();
    for model in without_sentinel.into_iter().chain(proxy_models) {
        if !resolved.contains(&model) {
            resolved.push(model);
        }
    }
    resolved
}
