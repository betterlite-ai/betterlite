const CMD_PERCENT_GUARD: &str = "%%cd:~,%";

fn double_trailing_backslashes(segment: &str) -> String {
    let bare = segment.trim_end_matches('\\');
    let trailing = segment.len().saturating_sub(bare.len());
    let mut doubled = bare.to_owned();
    doubled.push_str(&"\\".repeat(trailing.saturating_mul(2)));
    doubled
}

pub fn quote_for_cmd(token: &str) -> String {
    let escaped = token
        .split('"')
        .map(double_trailing_backslashes)
        .collect::<Vec<String>>()
        .join("\"\"")
        .replace('%', CMD_PERCENT_GUARD);
    format!("\"{escaped}\"")
}

mod tests;
