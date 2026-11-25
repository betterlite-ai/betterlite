use http::HeaderValue;

// Starlette decodes raw header bytes as latin-1, so every byte stands for one char
#[must_use]
pub fn header_text(value: &HeaderValue) -> String {
    value.as_bytes().iter().copied().map(char::from).collect()
}
