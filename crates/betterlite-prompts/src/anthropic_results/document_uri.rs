const DATA_PREFIX: &str = "data:";
// Anthropic's base64 document source accepts only these two mimes; other data URIs stay on the image path
const DOCUMENT_MEDIA_TYPES: [&str; 2] = ["application/pdf", "text/plain"];

#[must_use]
pub fn is_anthropic_document_data_uri(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(DATA_PREFIX) else {
        return false;
    };
    let end = rest.find([';', ',']).unwrap_or(rest.len());
    let media_type = rest.get(..end).unwrap_or_default();
    DOCUMENT_MEDIA_TYPES.contains(&media_type)
}
