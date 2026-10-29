use super::json::Json;

fn unmatched_openers(text: &str) -> Vec<char> {
    let mut opener_stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escape_next = false;
    for ch in text.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' {
            if in_string {
                escape_next = true;
            }
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' => opener_stack.push('}'),
            '[' => opener_stack.push(']'),
            '}' | ']' if opener_stack.last() == Some(&ch) => {
                opener_stack.pop();
            }
            _ => {}
        }
    }
    opener_stack
}

// Repairs only a trailing comma plus unmatched brackets; a balanced or still
// unparseable candidate falls through to None, never a broader rewrite.
pub(super) fn attempt_json_repair(text: &str) -> Option<Json> {
    let stripped = text.trim_end();
    if stripped.is_empty() {
        return None;
    }
    let mut opener_stack = unmatched_openers(stripped);
    if opener_stack.is_empty() {
        return None;
    }
    let mut candidate = stripped.trim_end_matches(',').to_owned();
    while let Some(closer) = opener_stack.pop() {
        candidate.push(closer);
    }
    serde_json::from_str(&candidate).ok()
}

mod tests;
