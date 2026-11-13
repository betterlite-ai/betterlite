#[must_use]
pub fn filter_for_signature(headers: &[(String, Option<String>)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let present = value.as_ref()?;
            let lower = name.to_ascii_lowercase();
            let signed = matches!(lower.as_str(), "host" | "content-type" | "date")
                || lower.starts_with("x-amz-")
                || lower.starts_with("x-amzn-");
            signed.then_some((name.clone(), present.clone()))
        })
        .collect()
}
pub fn reattach(
    original: &[(String, Option<String>)],
    signed: &mut Vec<(String, String)>,
    has_token: bool,
) {
    let kept = original
        .iter()
        .filter(|(name, _)| !computed(name, has_token));
    signed.extend(kept.filter_map(|(name, value)| Some((name.clone(), value.clone()?))));
}
// botocore only replaces a stale x-amz-security-token when the credentials carry one
const fn computed(name: &str, has_token: bool) -> bool {
    if name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("x-amz-date")
        || name.eq_ignore_ascii_case("date")
    {
        return true;
    }
    has_token && name.eq_ignore_ascii_case("x-amz-security-token")
}
#[must_use]
pub fn resolved(headers: &[(String, Option<String>)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), value.as_ref()?.clone())))
        .collect()
}
pub fn override_authorization(signed: &mut Vec<(String, String)>, supplied: Option<&str>) {
    if let Some(value) = supplied.filter(|value| !value.starts_with("AWS4-HMAC-SHA256")) {
        replace(signed, "Authorization".to_owned(), value.to_owned());
    }
}
pub fn set_exact(headers: &mut Vec<(String, Option<String>)>, name: &str, value: String) {
    match headers.iter_mut().find(|(existing, _)| existing == name) {
        Some((_, stored)) => *stored = Some(value),
        None => headers.push((name.to_owned(), Some(value))),
    }
}
pub fn replace(headers: &mut Vec<(String, String)>, name: String, value: String) {
    match headers
        .iter_mut()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(&name))
    {
        Some((_, stored)) => *stored = value,
        None => headers.push((name, value)),
    }
}
#[must_use]
pub fn without_computed(
    mut headers: Vec<(String, String)>,
    has_token: bool,
) -> Vec<(String, String)> {
    headers.retain(|(name, _)| !computed(name, has_token));
    headers
}
#[must_use]
pub fn merge_computed(
    mut headers: Vec<(String, String)>,
    computed: Vec<(String, String)>,
) -> Vec<(String, String)> {
    for (name, value) in computed {
        replace(&mut headers, name, value);
    }
    headers
}
#[must_use]
pub fn prepare(
    headers: Vec<(String, String)>,
    content_length: usize,
    method: &str,
) -> Vec<(String, String)> {
    let mut prepared: Vec<(String, String)> = Vec::with_capacity(headers.len().saturating_add(1));
    for (name, value) in headers {
        replace(&mut prepared, name, value);
    }
    let bounded = prepared.iter().any(|(name, _)| {
        name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
    });
    if !bounded && !matches!(method, "GET" | "HEAD" | "OPTIONS") {
        prepared.push(("Content-Length".to_owned(), content_length.to_string()));
    }
    prepared
}
