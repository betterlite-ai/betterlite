pub(crate) fn normalize_escaped_newlines(text: &str) -> String {
    if text.is_empty() || (!text.contains(r"\n") && !text.contains(r"\r")) {
        return text.to_owned();
    }
    if text.contains('\n') || text.contains('\r') {
        return text.to_owned();
    }
    text.replace(r"\r\n", "\n")
        .replace(r"\n", "\n")
        .replace(r"\r", "\n")
}
