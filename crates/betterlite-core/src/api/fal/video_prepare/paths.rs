pub const QUEUE_NAMESPACES: [&str; 2] = ["workflows", "comfy"];

const REQUESTS_SEGMENT: &str = "requests";

#[must_use]
pub fn queue_request_base_path(model: &str) -> String {
    let segments: Vec<&str> = model.split('/').collect();
    let count = match segments.first() {
        Some(first) if QUEUE_NAMESPACES.contains(first) => 3,
        Some(_) | None => 2,
    };
    let kept: Vec<&str> = segments.iter().take(count).copied().collect();
    kept.join("/")
}

#[must_use]
pub fn model_path_from_request_url(path: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let request_index = segments
        .iter()
        .position(|segment| *segment == REQUESTS_SEGMENT)?;
    let model_segments = segments.get(..request_index)?;
    let third_last = model_segments
        .len()
        .checked_sub(3)
        .and_then(|index| model_segments.get(index));
    let count = match third_last {
        Some(namespace) if QUEUE_NAMESPACES.contains(namespace) => 3,
        Some(_) | None => 2,
    };
    if model_segments.len() < count {
        return None;
    }
    let start = model_segments.len().checked_sub(count)?;
    Some(model_segments.get(start..)?.join("/"))
}

#[must_use]
pub fn request_id_from_request_url(path: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let request_index = segments
        .iter()
        .position(|segment| *segment == REQUESTS_SEGMENT)?;
    let request_segment = segments.get(request_index.saturating_add(1))?;
    Some((*request_segment).to_owned())
}
