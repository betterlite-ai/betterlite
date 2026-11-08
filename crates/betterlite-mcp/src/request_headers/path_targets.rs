mod tests;

use regex::Regex;

use super::client_headers::split_list;

const SSE_TARGET_PATHS: [&str; 2] = ["/mcp/sse", "/mcp/sse/messages"];
const MCP_PATH_PATTERN: &str = r"^/mcp/([^?#]+)(?:\?.*)?(?:#.*)?$";
const COMMA_TAIL_PATTERN: &str = "/([^/,]+(?:/[^/,]+)*)$";
const SINGLE_SERVER_PATTERN: &str = "^([^/]+(?:/[^/]+)?)(?:/.*)?$";

#[must_use]
pub fn target_names_from_path(path: &str) -> Vec<String> {
    if SSE_TARGET_PATHS.contains(&path.trim_end_matches('/')) {
        return Vec::new();
    }
    if let Some(first) = server_first_target(path) {
        return vec![first];
    }
    let Some(servers_and_path) = mcp_path_segment(path) else {
        return Vec::new();
    };
    if servers_and_path.contains(',') {
        return comma_separated_targets(&servers_and_path);
    }
    single_server_targets(&servers_and_path)
}

#[must_use]
pub fn resolve_target_server_names(path: &str, header_servers: Option<&[String]>) -> Vec<String> {
    let path_targets = target_names_from_path(path);
    if !path_targets.is_empty() {
        return path_targets;
    }
    header_servers.map_or_else(Vec::new, <[String]>::to_vec)
}

#[must_use]
pub fn cold_start_server_names(path: &str, header_servers: Option<&[String]>) -> Option<String> {
    let targets = target_names_from_path(path);
    let [target] = targets.as_slice() else {
        return None;
    };
    if let Some(servers) = header_servers
        && servers.iter().any(|name| name != target)
    {
        return None;
    }
    Some(target.clone())
}

fn server_first_target(path: &str) -> Option<String> {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next()?;
    let second = segments.next()?;
    (second == "mcp" && first != "mcp").then(|| first.to_owned())
}

fn mcp_path_segment(path: &str) -> Option<String> {
    let captures = Regex::new(MCP_PATH_PATTERN).ok()?.captures(path)?;
    let text = captures.get(1)?.as_str();
    (!text.is_empty()).then(|| text.to_owned())
}

fn comma_separated_targets(servers_and_path: &str) -> Vec<String> {
    let Ok(tail_pattern) = Regex::new(COMMA_TAIL_PATTERN) else {
        return Vec::new();
    };
    let servers_part = tail_pattern
        .find(servers_and_path)
        .map_or(servers_and_path, |tail| {
            servers_and_path.get(..tail.start()).unwrap_or_default()
        });
    split_list(servers_part)
}

fn single_server_targets(servers_and_path: &str) -> Vec<String> {
    let Ok(single_pattern) = Regex::new(SINGLE_SERVER_PATTERN) else {
        return Vec::new();
    };
    if let Some(single) = single_pattern
        .captures(servers_and_path)
        .and_then(|found| found.get(1))
    {
        return vec![single.as_str().to_owned()];
    }
    vec![servers_and_path.to_owned()]
}
