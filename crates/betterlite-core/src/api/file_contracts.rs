mod tests;

pub const BATCH_JSONL_CONTENT_TYPES: [&str; 6] = [
    "application/jsonl",
    "application/json",
    "application/octet-stream",
    "application/x-ndjson",
    "application/x-jsonlines",
    "text/plain",
];

#[must_use]
pub fn valid_content_type(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|candidate| {
        // LiteLLM compares the declared string verbatim, without lowercasing or dropping parameters
        BATCH_JSONL_CONTENT_TYPES.contains(&candidate)
    })
}

#[must_use]
pub fn is_batch_jsonl_file(
    purpose: Option<&str>,
    content_type: Option<&str>,
    content: Option<&[u8]>,
) -> bool {
    purpose.is_some_and(|value| value == "batch")
        && valid_content_type(content_type)
        && content.is_some()
}

#[must_use]
pub fn is_batch_jsonl_request<File>(
    purpose: Option<&str>,
    content_type: Option<&str>,
    file: Option<&File>,
) -> bool {
    if purpose != Some("batch") {
        return false;
    }
    if !valid_content_type(content_type) {
        return false;
    }
    file.is_some()
}
