use std::collections::BTreeSet;

pub struct DeploymentHint<'source> {
    pub model_name: &'source str,
    pub custom_llm_provider: Option<&'source str>,
    pub model: Option<&'source str>,
}

pub const PROVIDER: &str = "nvidia_nim";
pub const MODEL_PREFIX: &str = "nvidia_nim/";
const ROUTE_PREFIX: &str = "/nvidia_nim/";

#[must_use]
pub fn model_groups(deployments: &[DeploymentHint<'_>]) -> BTreeSet<String> {
    let mut nim_groups = BTreeSet::new();
    let mut other_groups = BTreeSet::new();
    for deployment in deployments {
        let is_nim = deployment.custom_llm_provider == Some(PROVIDER)
            || deployment.model.unwrap_or("").starts_with(MODEL_PREFIX);
        let groups = if is_nim {
            &mut nim_groups
        } else {
            &mut other_groups
        };
        groups.insert(deployment.model_name.to_owned());
    }
    nim_groups.difference(&other_groups).cloned().collect()
}

#[must_use]
pub fn model_group_in_path(path: &str, deployments: &[DeploymentHint<'_>]) -> Option<String> {
    let stripped = match path.get(..ROUTE_PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(ROUTE_PREFIX) => {
            path.get(ROUTE_PREFIX.len()..).unwrap_or_default()
        }
        _ => path,
    };
    router_model_in_endpoint(stripped, &model_groups(deployments))
}

#[must_use]
pub fn router_model_in_endpoint(
    endpoint: &str,
    router_models: &BTreeSet<String>,
) -> Option<String> {
    let segments: Vec<&str> = endpoint
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    for length in (1..=segments.len()).rev() {
        let candidate = segments.get(..length).unwrap_or_default().join("/");
        if router_models.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}
