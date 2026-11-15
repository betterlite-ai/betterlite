// object keys are POSIX names, so this follows posixpath rather than a file-system path type
fn posix_normpath(path: &str) -> String {
    let leading_slashes = path
        .chars()
        .take_while(|character| *character == '/')
        .count();
    let initial_slashes = if leading_slashes >= 2 && !path.starts_with("///") {
        2
    } else if leading_slashes >= 1 {
        1
    } else {
        0
    };
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        let keep = component != ".."
            || (initial_slashes == 0 && components.is_empty())
            || components.last() == Some(&"..");
        if keep {
            components.push(component);
        } else if !components.is_empty() {
            components.pop();
        }
    }
    let joined = components.join("/");
    if initial_slashes == 0 {
        if joined.is_empty() {
            ".".to_owned()
        } else {
            joined
        }
    } else {
        let prefix = "/".repeat(initial_slashes);
        format!("{prefix}{joined}")
    }
}

fn posix_dirname(path: &str) -> &str {
    let Some(index) = path.rfind('/') else {
        return "";
    };
    let Some(head) = path.get(..index.saturating_add(1)) else {
        return "";
    };
    if head.chars().all(|character| character == '/') {
        head
    } else {
        head.trim_end_matches('/')
    }
}

fn posix_join(directory: &str, entry: &str) -> String {
    if directory.is_empty() || directory.ends_with('/') {
        return format!("{directory}{entry}");
    }
    format!("{directory}/{entry}")
}

#[must_use]
pub fn resolve_include_object_key(config_object_key: &str, include_entry: &str) -> String {
    if include_entry.starts_with('/') {
        return posix_normpath(include_entry)
            .trim_start_matches('/')
            .to_owned();
    }
    posix_normpath(&posix_join(posix_dirname(config_object_key), include_entry))
}

mod tests;
