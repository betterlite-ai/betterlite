use super::wire::Message;

#[must_use]
pub fn texts(
    masked: &[String],
    originals: &[String],
    scanned_slice: Option<(usize, usize)>,
    scanned_role_subset: bool,
) -> Vec<String> {
    if masked.is_empty() {
        return originals.to_vec();
    }
    if let Some((offset, length)) = scanned_slice {
        let mut merged = originals.to_vec();
        for (slot, text) in merged.iter_mut().skip(offset).take(length).zip(masked) {
            slot.clone_from(text);
        }
        return merged;
    }
    if scanned_role_subset {
        tracing::warn!(
            "could not align masked texts with request texts, skipping masking write-back"
        );
        originals.to_vec()
    } else {
        masked.to_vec()
    }
}

#[must_use]
pub fn messages(
    original: Option<&[Message]>,
    updated: &[Message],
    indices: Option<&[usize]>,
) -> Vec<Message> {
    let Some(targets) = indices.filter(|items| !items.is_empty()) else {
        return updated.to_vec();
    };
    let mut merged = original
        .filter(|items| !items.is_empty())
        .unwrap_or(updated)
        .to_vec();
    for (index, message) in targets.iter().zip(updated) {
        if let Some(slot) = merged.get_mut(*index) {
            slot.clone_from(message);
        }
    }
    merged
}
