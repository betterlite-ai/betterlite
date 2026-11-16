use super::QueuedItem;

// each line is the payload as JSON, and the join leaves no trailing newline
pub fn combine_payloads_to_ndjson(items: &[QueuedItem]) -> Result<String, serde_json::Error> {
    let mut lines: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        lines.push(serde_json::to_string(item.payload.as_ref())?);
    }
    Ok(lines.join("\n"))
}

mod tests;
