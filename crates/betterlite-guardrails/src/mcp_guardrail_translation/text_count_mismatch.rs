use super::error::{ContentBlocked, ScanTarget};

mod tests;

// a guardrail that drops or adds texts would make the positional write-back mask the wrong value
pub fn masked_texts(
    returned: Option<Vec<String>>,
    scanned: usize,
    target: ScanTarget,
) -> Result<Option<Vec<String>>, ContentBlocked> {
    match returned {
        None => Ok(None),
        Some(texts) if texts.len() == scanned => Ok(Some(texts)),
        Some(texts) => Err(ContentBlocked::TextCount {
            returned: texts.len(),
            scanned,
            target,
        }),
    }
}
