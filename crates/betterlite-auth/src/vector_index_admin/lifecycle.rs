mod tests;

#[must_use]
pub fn is_index_lifecycle_request(method: &str, path: &str, index_name: &str) -> bool {
    if !matches!(method, "POST" | "PUT" | "DELETE" | "PATCH") {
        return false;
    }
    if let Some(suffix) = suffix_after_index_name(path, index_name) {
        if suffix.starts_with("/docs") {
            return false;
        }
        if suffix.is_empty() || suffix.starts_with('?') {
            return true;
        }
    }
    let route = path.split_once('?').map_or(path, |(before, _)| before);
    method == "POST" && route.trim_end_matches('/').ends_with("/indexes")
}

fn suffix_after_index_name<'path>(path: &'path str, index_name: &str) -> Option<&'path str> {
    let needle = format!("/indexes/{index_name}");
    path.match_indices(needle.as_str())
        .filter_map(|(start, _)| {
            let end = start.checked_add(needle.len())?;
            path.get(end..)
        })
        .find(|suffix| suffix.is_empty() || suffix.starts_with('/') || suffix.starts_with('?'))
}
