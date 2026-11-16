const DEPLOYMENT_PREFIX: &str = "openai/deployments/";

// LiteLLM's AZURE_BODY_MODEL_INFERENCE_ENDPOINTS; a path ending in one of these carries its model in the body
const BODY_MODEL_INFERENCE_ENDPOINTS: [&str; 6] = [
    "responses",
    "chat/completions",
    "completions",
    "embeddings",
    "images/generations",
    "audio/speech",
];

// AZURE_DEPLOYMENT_SEGMENT, spelled out: "openai/deployments/" at the start or after a slash, then a nonempty segment
fn deployment_segment(endpoint: &str) -> Option<&str> {
    for (index, _prefix) in endpoint.match_indices(DEPLOYMENT_PREFIX) {
        let at_start_or_slash = endpoint
            .as_bytes()
            .get(index.wrapping_sub(1))
            .is_none_or(|byte| *byte == b'/');
        if !at_start_or_slash {
            continue;
        }
        let Some(after) = endpoint
            .get(index..)
            .and_then(|tail| tail.strip_prefix(DEPLOYMENT_PREFIX))
        else {
            continue;
        };
        let segment = after.split('/').next().unwrap_or_default();
        if !segment.is_empty() {
            return Some(segment);
        }
    }
    None
}

#[must_use]
pub fn body_model_inference_endpoint(endpoint: &str) -> bool {
    if deployment_segment(endpoint).is_some() {
        return false;
    }
    let path = endpoint.trim_matches('/');
    BODY_MODEL_INFERENCE_ENDPOINTS
        .iter()
        .any(|name| path == *name || path.ends_with(&format!("/{name}")))
}

// the parts include the empty strings a leading, trailing or doubled slash leaves, as str.split does
#[must_use]
pub fn router_model_in_endpoint<'endpoint>(
    endpoint: &'endpoint str,
    router_models: &[String],
) -> Option<&'endpoint str> {
    let parts: Vec<&str> = endpoint.split('/').collect();
    if parts.len() < 2 {
        return None;
    }
    parts
        .into_iter()
        .find(|part| router_models.iter().any(|model| model == *part))
}

#[must_use]
pub fn foreign_azure_deployment(
    endpoint: &str,
    model_group: &str,
    served_models: impl FnOnce() -> Vec<String>,
) -> Option<String> {
    let deployment = deployment_segment(endpoint)?;
    if deployment == model_group {
        return None;
    }
    let folded = caseless::default_case_fold_str(deployment);
    if served_models()
        .iter()
        .any(|name| caseless::default_case_fold_str(name) == folded)
    {
        return None;
    }
    Some(deployment.to_owned())
}
