use url::form_urlencoded;

pub(super) fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

// httpx's dict(URL.params) keeps the first value of each name in first-occurrence order
pub(super) fn first_query_values(query: Option<&str>) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for (name, value) in query_pairs(query) {
        if !kept.iter().any(|(kept_name, _)| *kept_name == name) {
            kept.push((name, value));
        }
    }
    kept
}

pub(super) fn encoded_query(pairs: &[(String, String)]) -> Option<String> {
    if pairs.is_empty() {
        return None;
    }
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(name, value);
    }
    // Python's urlencode uses quote_plus: it encodes * and keeps ~, the reverse of form_urlencoded
    Some(serializer.finish().replace('*', "%2A").replace("%7E", "~"))
}
