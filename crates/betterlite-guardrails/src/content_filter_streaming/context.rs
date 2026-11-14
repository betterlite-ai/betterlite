use crate::content_filter_streaming::plan::StreamedScanPlan;

mod tests;

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
    // only the last part before the cut and the first part after it meet, as re.split gives them
    let cut_head = head_lower
        .split(['.', '!', '?'])
        .next_back()
        .unwrap_or_default();
    let cut_tail = tail_lower.split(['.', '!', '?']).next().unwrap_or_default();
    let cut_sentence = format!("{cut_head}{cut_tail}");
    plan.conditional_words
        .iter()
        .any(|word| cut_sentence.contains(word.as_str()))
}
