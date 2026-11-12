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
    let Some(candidate) = content_type else {
        return false;
    };
    BATCH_JSONL_CONTENT_TYPES.contains(&candidate)
}

#[must_use]
pub fn is_batch_jsonl_file(
    purpose: Option<&str>,
    content_type: Option<&str>,
    content: Option<&[u8]>,
) -> bool {
    match content {
        Some(_) => purpose == Some("batch") && valid_content_type(content_type),
        None => false,
    }
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
    valid_content_type(content_type) && file.is_some()
}
