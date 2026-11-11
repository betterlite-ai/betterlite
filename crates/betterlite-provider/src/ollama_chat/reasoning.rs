const OPEN_TAGS: [&str; 3] = ["<think>", "<thinking>", "<budget:thinking>"];
const CLOSE_TAGS: [&str; 3] = ["</think>", "</thinking>", "</budget:thinking>"];

// LiteLLM's _parse_content_for_reasoning: a leading reasoning tag yields the reasoning and the text after the first closing tag
#[must_use]
pub fn parse_content_for_reasoning(text: &str) -> (Option<&str>, &str) {
    if text.is_empty() {
        return (None, text);
    }
    let Some(rest) = OPEN_TAGS.iter().find_map(|tag| text.strip_prefix(tag)) else {
        return (None, text);
    };
    let Some((start, length)) = CLOSE_TAGS
        .iter()
        .filter_map(|tag| rest.find(tag).map(|start| (start, tag.len())))
        .min_by_key(|(start, _length)| *start)
    else {
        return (None, text);
    };
    let reasoning = rest.get(..start).unwrap_or_default();
    let content = rest.get(start.saturating_add(length)..).unwrap_or_default();
    (Some(reasoning), content)
}
