#[must_use]
pub fn strip_sse_data_from_chunk(chunk: Option<&str>) -> Option<&str> {
    let line = chunk?;
    if let Some(rest) = line.strip_prefix("data: ") {
        return Some(rest);
    }
    if let Some(rest) = line.strip_prefix("data:") {
        return Some(rest);
    }
    Some(line)
}
