use std::cmp::Reverse;

pub fn normalize_root_paths(raw_paths: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for entry in raw_paths {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with('/') {
            tracing::warn!(
                "SERVER_ROOT_PATHS entry {entry:?} does not start with '/' and will be ignored."
            );
            continue;
        }
        let prefix = trimmed.trim_end_matches('/');
        if prefix.is_empty() {
            tracing::warn!(
                "SERVER_ROOT_PATHS entry {entry:?} is the bare root and will be ignored; a root-mounted deployment needs no entry."
            );
            continue;
        }
        if !kept.iter().any(|existing| existing == prefix) {
            kept.push(prefix.to_owned());
        }
    }
    kept.sort_by_key(|prefix| Reverse(prefix.chars().count()));
    kept
}
