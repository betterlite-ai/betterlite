mod tests;

// Real newlines mean the text was never JSON-escaped, so a literal \n there is content
#[must_use]
pub fn normalize_escaped_newlines(text: &str) -> String {
    if !(text.contains(r"\n") || text.contains(r"\r")) || text.contains(['\n', '\r']) {
        return text.to_owned();
    }
    text.replace(r"\r\n", "\n")
        .replace(r"\n", "\n")
        .replace(r"\r", "\n")
}
