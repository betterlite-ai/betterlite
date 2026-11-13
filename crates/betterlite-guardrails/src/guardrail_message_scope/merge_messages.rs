use std::collections::{HashMap, HashSet};

mod tests;

// A guardrail sees only the scoped messages, so its answer is swapped back in place; extra messages follow the last scoped one and scoped originals without a counterpart are dropped.
#[must_use]
pub fn merge_scoped_messages<Message>(
    full_messages: &[Message],
    scoped_indices: &[usize],
    guardrailed_scoped: &[Message],
) -> Vec<Message>
where
    Message: Clone,
{
    let replacements: HashMap<usize, &Message> = scoped_indices
        .iter()
        .copied()
        .zip(guardrailed_scoped)
        .collect();
    let removed: HashSet<usize> = scoped_indices
        .iter()
        .skip(guardrailed_scoped.len())
        .copied()
        .collect();
    let appended = guardrailed_scoped
        .get(scoped_indices.len()..)
        .unwrap_or_default();
    let last_scoped_index = scoped_indices.last().copied();
    let mut merged = Vec::new();
    for (index, message) in full_messages.iter().enumerate() {
        if removed.contains(&index) {
            continue;
        }
        merged.push(replacements.get(&index).copied().unwrap_or(message).clone());
        if Some(index) == last_scoped_index {
            merged.extend_from_slice(appended);
        }
    }
    merged
}
