const DATA_URL_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64";

// Python's email Message.get_content_type: the part before the first ';', trimmed and lowered, only when it names one type/subtype
#[must_use]
pub fn parse_content_type(content_type: &str) -> String {
    let stated = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if stated.matches('/').count() == 1 {
        stated
    } else {
        "text/plain".to_owned()
    }
}

// Python's re.match(r"data:(.*?);base64"): the first marker on the line, since '.' never crosses a newline
#[must_use]
pub fn parse_mime_type(base64_data: &str) -> Option<&str> {
    let body = base64_data.strip_prefix(DATA_URL_PREFIX)?;
    let marker = body.find(BASE64_MARKER)?;
    let mime_type = body.get(..marker)?;
    (!mime_type.contains('\n')).then_some(mime_type)
}
