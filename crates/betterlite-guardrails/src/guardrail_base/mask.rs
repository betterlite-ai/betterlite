#[must_use]
pub fn mask_content_in_string(content: &str, mask: &str, start: i64, end: i64) -> String {
    // Python slices by code point, and a provider sends the offsets as plain signed numbers
    let (Ok(from), Ok(to)) = (usize::try_from(start), usize::try_from(end)) else {
        return content.to_owned();
    };
    if from >= to || to > content.chars().count() {
        return content.to_owned();
    }
    let head: String = content.chars().take(from).collect();
    let tail: String = content.chars().skip(to).collect();
    format!("{head}{mask}{tail}")
}
