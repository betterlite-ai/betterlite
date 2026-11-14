use crate::content_filter_streaming::plan::StreamedScanPlan;

mod tests;

// Python's re.split(r"[.!?]+", text): the parts between runs of sentence terminators
fn split_sentences(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_terminators = false;
    for (offset, character) in text.char_indices() {
        if matches!(character, '.' | '!' | '?') {
            if !in_terminators {
                parts.push(text.get(start..offset).unwrap_or_default());
                in_terminators = true;
            }
        } else if in_terminators {
            in_terminators = false;
            start = offset;
        }
    }
    // a text that ends in terminators splits into an empty last part, as re.split does
    if in_terminators {
        parts.push("");
    } else {
        parts.push(text.get(start..).unwrap_or_default());
    }
    parts
}

#[must_use]
pub fn cut_breaks_wider_context(
    buffered_text: &str,
    head: &str,
    tail: &str,
    plan: &StreamedScanPlan,
) -> bool {
    let buffered_lower = buffered_text.to_lowercase();
    let tail_lower = tail.to_lowercase();
    let phrase_leaves = plan.exception_phrases.iter().any(|phrase| {
        buffered_lower.contains(phrase.as_str()) && !tail_lower.contains(phrase.as_str())
    });
    if phrase_leaves {
        return true;
    }
    let head_lower = head.to_lowercase();
    let cut_head = split_sentences(&head_lower)
        .last()
        .copied()
        .unwrap_or_default();
    let cut_tail = split_sentences(&tail_lower)
        .first()
        .copied()
        .unwrap_or_default();
    let cut_sentence = format!("{cut_head}{cut_tail}");
    plan.conditional_words
        .iter()
        .any(|word| cut_sentence.contains(word.as_str()))
}
