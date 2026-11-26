use regex::Regex;

// the scheme prefix a presented key carries, or the SigV4 credential inside an AWS authorization header
pub(super) fn strip_prefix(value: &str) -> Option<String> {
    for prefix in ["Bearer ", "Basic ", "bearer "] {
        if value.starts_with(prefix) {
            return Some(value.replace(prefix, ""));
        }
    }
    value
        .starts_with("AWS4-HMAC-SHA256")
        .then(|| aws_credential(value))
        .flatten()
}

fn aws_credential(value: &str) -> Option<String> {
    let bearer = Regex::new(r"Credential=Bearer\s+([^/\s,]+)").ok()?;
    let plain = Regex::new(r"Credential=([^/\s,]+)").ok()?;
    let captures = bearer.captures(value).or_else(|| plain.captures(value))?;
    captures.get(1).map(|token| token.as_str().to_owned())
}
