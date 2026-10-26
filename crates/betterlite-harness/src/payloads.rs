#![expect(
    clippy::disallowed_types,
    reason = "Harness schemas are arbitrary JSON documents"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{MAIN_SEPARATOR, Path};
use std::slice::from_ref;
use std::str::from_utf8;

use serde::Deserialize as _;
use serde_json::{Map, Value};

use crate::types::JsonObject;

pub const SKILL_MANIFEST: &str = "SKILL.md";
const MAX_RECURSE_DEPTH: usize = 100;

#[must_use]
pub fn normalize_tool_name(native_name: &str, mapping: &BTreeMap<String, String>) -> String {
    mapping
        .get(native_name)
        .cloned()
        .unwrap_or_else(|| native_name.to_owned())
}

#[must_use]
pub fn native_tool_names(
    normalized: &[String],
    mapping: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut names = Vec::new();
    for name in normalized {
        let candidates: &[String] = mapping
            .get(name)
            .map_or_else(|| from_ref(name), |values| values.as_slice());
        for candidate in candidates {
            if !names.contains(candidate) {
                names.push(candidate.clone());
            }
        }
    }
    names
}

#[must_use]
pub fn last_json_object(text: &str) -> Option<String> {
    let mut last = None;
    for (index, _) in text
        .char_indices()
        .filter(|(_, character)| *character == '{')
    {
        if let Some(fragment) = text.get(index..)
            && let Ok(value) = Value::deserialize(&mut serde_json::Deserializer::from_str(fragment))
            && value.is_object()
        {
            last = serde_json::to_string(&value).ok();
        }
    }
    last
}

#[must_use]
pub fn structured_output_instruction(schema: &Value) -> String {
    let encoded = serde_json::to_string(schema).unwrap_or_else(|_| "null".to_owned());
    format!(
        "When you have finished, your final message must be a single JSON object that matches this JSON schema, with no other text before or after it:\n{encoded}"
    )
}

pub fn strict_json_schema(schema: &Value) -> Result<Value, String> {
    strict_json_schema_at(schema, 0)
}

fn strict_json_schema_at(schema: &Value, depth: usize) -> Result<Value, String> {
    if depth > MAX_RECURSE_DEPTH {
        return Err(format!(
            "output schema is nested deeper than {MAX_RECURSE_DEPTH} levels"
        ));
    }
    match schema {
        Value::Array(entries) => entries
            .iter()
            .map(|entry| strict_json_schema_at(entry, depth.saturating_add(1)))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(entries) => strict_object_schema(entries, depth),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(schema.clone()),
    }
}

fn strict_object_schema(entries: &Map<String, Value>, depth: usize) -> Result<Value, String> {
    if let Some(reference) = entries.get("$ref") {
        let mut result = Map::new();
        result.insert("$ref".to_owned(), reference.clone());
        return Ok(Value::Object(result));
    }
    let mut result = Map::new();
    for (key, value) in entries {
        if key != "default" {
            result.insert(
                key.clone(),
                strict_json_schema_at(value, depth.saturating_add(1))?,
            );
        }
    }
    if result.get("type") == Some(&Value::String("object".to_owned()))
        || result.get("properties").is_some_and(Value::is_object)
    {
        let properties = result
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let required = properties.keys().cloned().map(Value::String).collect();
        result.insert("properties".to_owned(), Value::Object(properties));
        result.insert("required".to_owned(), Value::Array(required));
        result.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
    Ok(Value::Object(result))
}

#[must_use]
pub fn decode_json_line<Input>(line: Input) -> Option<JsonObject>
where
    Input: AsRef<[u8]>,
{
    let text = from_utf8(line.as_ref()).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(object) => Some(object.into_iter().collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            None
        }
    }
}

#[must_use]
pub fn stderr_tail_text(lines: &[String]) -> String {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn read_skill_files<Root>(skill_root: Root) -> io::Result<Vec<(String, Vec<u8>)>>
where
    Root: AsRef<Path>,
{
    let root = skill_root.as_ref();
    let manifest = root.join(SKILL_MANIFEST);
    if !manifest.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("skill folder has no {SKILL_MANIFEST}"),
        ));
    }
    let mut files = Vec::new();
    collect_skill_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn collect_skill_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> io::Result<()> {
    for directory_entry in fs::read_dir(directory)? {
        let entry = directory_entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_skill_files(root, &path, files)?;
            continue;
        }
        if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| io::Error::other(error.to_string()))?
                .to_str()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "path is not valid UTF-8")
                })?
                .replace(MAIN_SEPARATOR, "/");
            files.push((relative, fs::read(path)?));
        }
    }
    Ok(())
}

#[cfg(test)]
#[expect(
    clippy::inline_modules,
    reason = "Behavior tests are kept beside the helper implementation"
)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_expand_and_deduplicate() {
        let mapping = BTreeMap::from([(String::from("read"), vec![String::from("cat")])]);
        assert_eq!(
            native_tool_names(&[String::from("read"), String::from("cat")], &mapping),
            ["cat"]
        );
    }

    #[test]
    fn payload_helpers_decode_last_object_and_jsonl() {
        assert_eq!(
            last_json_object("noise {\"a\":1} tail"),
            Some(String::from("{\"a\":1}"))
        );
        assert_eq!(
            decode_json_line(br#"{"event":"done"}"#)
                .and_then(|object| object.get("event").cloned()),
            Some(Value::String(String::from("done")))
        );
        assert!(decode_json_line("[]").is_none());
    }

    #[test]
    fn replays_the_payloads_corpus() {
        for bytes in [b"".as_slice(), br#"{"event":"done"}"#] {
            drop(decode_json_line(bytes));
        }
    }
}
