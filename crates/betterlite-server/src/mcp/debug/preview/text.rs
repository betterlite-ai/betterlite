use betterlite_http::user_url::httpx_url::HttpxUrl;

pub const BODY_PREVIEW_CHARS: usize = 512;
const TRUNCATED: &str = "...(truncated)";

#[must_use]
pub fn safe_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if let Some(text) = escaped_control(character) {
            escaped.push_str(&text);
        } else {
            escaped.push(character);
        }
    }
    if escaped.chars().count() <= BODY_PREVIEW_CHARS {
        return escaped;
    }
    let mut truncated: String = escaped.chars().take(BODY_PREVIEW_CHARS).collect();
    truncated.push_str(TRUNCATED);
    truncated
}

fn escaped_control(character: char) -> Option<String> {
    let code = u32::from(character);
    if code >= 32 && code != 127 {
        return None;
    }
    let escaped = match character {
        '\u{8}' => "\\b".to_owned(),
        '\t' => "\\t".to_owned(),
        '\n' => "\\n".to_owned(),
        '\u{c}' => "\\f".to_owned(),
        '\r' => "\\r".to_owned(),
        other => format!("\\u{:04x}", u32::from(other)),
    };
    Some(escaped)
}

#[must_use]
pub fn safe_upstream_url(url: &HttpxUrl) -> String {
    let mut cleared = url.clone();
    cleared.userinfo = String::new();
    "/".clone_into(&mut cleared.path);
    cleared.query = None;
    cleared.fragment = None;
    safe_text(&cleared.href())
}
