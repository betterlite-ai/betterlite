pub const ENDPOINT_ACTIONS: [&str; 6] = [
    "invoke",
    "invoke-with-response-stream",
    "converse",
    "converse-stream",
    "count_tokens",
    "count-tokens",
];
pub const STREAMING_ACTIONS: [&str; 2] = ["invoke-with-response-stream", "converse-stream"];

const MODEL_SEGMENT: &str = "model";
const PROFILE_SEGMENT: &str = "application-inference-profile";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    #[error(
        "'model' keyword not found and unable to extract model from endpoint. Expected format: /model/{{modelId}}/{{action}}. Got: {endpoint}"
    )]
    MissingModelKeyword { endpoint: String },
    #[error(
        "No model ID found after 'model' keyword. Expected format: /model/{{modelId}}/{{action}}. Got: {endpoint}"
    )]
    NoModelId { endpoint: String },
}

#[must_use]
pub fn is_count_tokens_endpoint(endpoint: &str) -> bool {
    let underscored = endpoint.contains("count_tokens");
    let hyphenated = endpoint.contains("count-tokens");
    underscored || hyphenated
}

pub fn extract_model(endpoint: &str) -> Result<String, EndpointError> {
    let parts: Vec<&str> = endpoint.split('/').collect();
    if endpoint.contains(PROFILE_SEGMENT) {
        return Ok(parts
            .iter()
            .skip(1)
            .take(2)
            .copied()
            .collect::<Vec<&str>>()
            .join("/"));
    }
    let action_index = parts
        .iter()
        .position(|part| ENDPOINT_ACTIONS.contains(part));
    let Some(model_index) = parts.iter().position(|part| *part == MODEL_SEGMENT) else {
        return model_without_marker(endpoint, &parts, action_index);
    };
    let after_model = model_index.saturating_add(1);
    model_after_marker(endpoint, &parts, after_model, action_index)
}

fn model_without_marker(
    endpoint: &str,
    parts: &[&str],
    action_index: Option<usize>,
) -> Result<String, EndpointError> {
    if let Some(index) = action_index.filter(|index| *index > 1) {
        let model_parts: Vec<&str> = parts
            .get(1..index)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|part| !part.is_empty())
            .collect();
        if !model_parts.is_empty() {
            return Ok(model_parts.join("/"));
        }
    }
    Err(EndpointError::MissingModelKeyword {
        endpoint: endpoint.to_owned(),
    })
}

fn model_after_marker(
    endpoint: &str,
    parts: &[&str],
    after_model: usize,
    action_index: Option<usize>,
) -> Result<String, EndpointError> {
    if let Some(index) = action_index.filter(|index| *index > after_model) {
        let model_parts: Vec<&str> = parts.get(after_model..index).unwrap_or_default().to_vec();
        return Ok(model_parts.join("/"));
    }
    let model_parts: Vec<&str> = parts
        .get(after_model..)
        .unwrap_or_default()
        .iter()
        .copied()
        .filter(|part| !part.is_empty())
        .collect();
    if model_parts.is_empty() {
        return Err(EndpointError::NoModelId {
            endpoint: endpoint.to_owned(),
        });
    }
    Ok(model_parts.join("/"))
}
