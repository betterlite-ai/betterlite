use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

fn code_keywords() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"\b(?:def |function |class |import |from |require\(|#include|fn |func |const |let |var |public |private |static )\b").ok())
        .as_ref()
}

pub fn detect_content_type(content: &str) -> &'static str {
    let stripped = content.trim();
    if stripped.is_empty() {
        return "text";
    }
    if matches!(stripped.as_bytes().first(), Some(b'{' | b'['))
        && serde_json::from_str::<Value>(stripped).is_ok()
    {
        return "json";
    }
    let sample = &stripped[..stripped.len().min(5000)];
    let keyword_matches = code_keywords().map_or(0, |regex| regex.find_iter(sample).count());
    let lines: Vec<&str> = sample.split('\n').collect();
    let indented = lines
        .iter()
        .filter(|line| {
            (line.starts_with("    ") || line.starts_with('\t')) && !line.trim().is_empty()
        })
        .count();
    if keyword_matches >= 3 || (indented * 10 > lines.len() * 3 && lines.len() > 5) {
        "code"
    } else {
        "text"
    }
}

#[cfg(test)]
mod tests;
