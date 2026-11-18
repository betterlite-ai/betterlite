use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// The router's get_model_info in the source: the model_info mapping of one deployment, or None when it cannot be found or carries none.
pub trait ServedDeployments: Send + Sync {
    fn model_info(&self, deployment_id: &str) -> Option<&JsonObject>;
}

// _served_model_access_groups: None keeps the auth-attributed set; a known model_info without a list of access_groups narrows it to nothing.
#[must_use]
pub fn served_model_access_groups(
    deployments: Option<&dyn ServedDeployments>,
    served_model_id: Option<&str>,
) -> Option<BTreeSet<String>> {
    let deployments = deployments?;
    let served_model_id = served_model_id.filter(|deployment_id| !deployment_id.is_empty())?;
    let model_info = deployments.model_info(served_model_id)?;
    let Some(Json::List(declared)) = model_info.get("access_groups") else {
        return Some(BTreeSet::new());
    };
    Some(
        declared
            .iter()
            .filter_map(|group| match group {
                Json::Text(name) => Some(name.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
    )
}

// debitable_model_access_groups: B1715's ordered coercion first, then the served deployment narrows it without ever adding a group.
#[must_use]
pub fn debitable_model_access_groups(
    attributed: &[String],
    served_model_id: Option<&str>,
    deployments: Option<&dyn ServedDeployments>,
) -> Vec<String> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut ordered: Vec<&String> = Vec::new();
    for group in attributed {
        if group.is_empty() || !seen.insert(group.as_str()) {
            continue;
        }
        ordered.push(group);
    }
    if ordered.is_empty() {
        return Vec::new();
    }
    match served_model_access_groups(deployments, served_model_id) {
        None => ordered.into_iter().cloned().collect(),
        Some(served) => ordered
            .into_iter()
            .filter(|group| served.contains(*group))
            .cloned()
            .collect(),
    }
}

#[path = "access_groups/tests.rs"]
mod tests;
