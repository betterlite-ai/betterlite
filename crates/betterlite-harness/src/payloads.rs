mod python_json;
mod tests;

use std::collections::BTreeMap;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::{fs, io, slice};

use thiserror::Error;

use crate::payloads::python_json::{OrderedJson, python_json_text};
use crate::types::JsonObject;

pub const SKILL_MANIFEST: &str = "SKILL.md";
pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("output schema is nested deeper than {0} levels")]
    SchemaTooDeep(usize),
    #[error("skill folder {} has no {SKILL_MANIFEST}", .0.display())]
    NoManifest(PathBuf),
    #[error("could not read {}", .0.display())]
    Read(PathBuf, #[source] io::Error),
    #[error("skill file {} has a name that is not UTF-8", .0.display())]
    PathNotUtf8(PathBuf),
    #[error("the JSON could not be written")]
    Encode(#[source] serde_json::Error),
}

#[must_use]
pub fn normalize_tool_name(native_name: &str, mapping: &BTreeMap<String, String>) -> String {
    mapping
        .get(native_name)
        .map_or(native_name, String::as_str)
        .to_owned()
}

#[must_use]
pub fn native_tool_names(
    normalized: &[String],
    mapping: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut names = Vec::new();
    for name in normalized {
        let native = mapping
            .get(name)
            .map_or_else(|| slice::from_ref(name), Vec::as_slice);
        for candidate in native {
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
    let mut start = text.find('{');
    while let Some(index) = start {
        let mut values =
            serde_json::Deserializer::from_str(text.get(index..)?).into_iter::<OrderedJson>();
        let encoded = values
            .next()
            .and_then(Result::ok)
            .and_then(|object| python_json_text(&object).ok());
        // a parsed object hides the braces inside it, a failed one only its own
        let resume = if encoded.is_some() {
            last = encoded;
            index.saturating_add(values.byte_offset())
        } else {
            index.saturating_add(1)
        };
        start = text
            .get(resume..)
            .and_then(|rest| rest.find('{'))
            .map(|offset| resume.saturating_add(offset));
    }
    last
}

pub fn structured_output_instruction(schema: &JsonObject) -> Result<String, PayloadError> {
    let schema_text = python_json_text(schema).map_err(PayloadError::Encode)?;
    Ok(format!(
        "When you have finished, your final message must be a single JSON object that matches this JSON schema, with no other text before or after it:\n{schema_text}"
    ))
}

#[expect(
    clippy::disallowed_types,
    reason = "an output schema is any JSON Schema document, and strict mode rewrites it key by key"
)]
pub fn strict_json_schema(
    schema: serde_json::Value,
    depth: usize,
) -> Result<serde_json::Value, PayloadError> {
    use serde_json::{Map, Value};
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Err(PayloadError::SchemaTooDeep(DEFAULT_MAX_RECURSE_DEPTH));
    }
    let nested = depth.saturating_add(1);
    let entries = match schema {
        Value::Array(items) => {
            return items
                .into_iter()
                .map(|item| strict_json_schema(item, nested))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array);
        }
        Value::Object(entries) => entries,
        scalar @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => {
            return Ok(scalar);
        }
    };
    let mut result = Map::new();
    for (key, value) in entries {
        result.insert(key, strict_json_schema(value, nested)?);
    }
    if result.contains_key("$ref") {
        result.retain(|key, _| key == "$ref");
        return Ok(Value::Object(result));
    }
    result.remove("default");
    let declared = result.get("properties").and_then(Value::as_object).cloned();
    if declared.is_some() || result.get("type").and_then(Value::as_str) == Some("object") {
        let properties = declared.unwrap_or_default();
        let required = properties.keys().cloned().map(Value::String).collect();
        result.insert("properties".to_owned(), Value::Object(properties));
        result.insert("required".to_owned(), Value::Array(required));
        result.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
    Ok(Value::Object(result))
}

#[must_use]
pub fn decode_json_line(line: &[u8]) -> Option<JsonObject> {
    // bytes.strip() also drops the vertical tab that u8::is_ascii_whitespace keeps
    let is_space = |byte: &u8| byte.is_ascii_whitespace() || *byte == 0x0b;
    let first = line.iter().position(|byte| !is_space(byte))?;
    let last = line.iter().rposition(|byte| !is_space(byte))?;
    let text = line.get(first..=last)?;
    // json.loads decodes bytes as utf-8-sig, so a leading byte order mark is dropped
    serde_json::from_slice(text.strip_prefix(UTF8_BOM).unwrap_or(text)).ok()
}

#[must_use]
pub fn stderr_tail_text<Lines>(stderr_tail: Lines) -> String
where
    Lines: IntoIterator,
    Lines::Item: AsRef<str>,
{
    let mut tail = String::new();
    for line in stderr_tail {
        let text = line.as_ref();
        // str.isspace() also counts the separators U+001C to U+001F
        if text
            .chars()
            .all(|character| character.is_whitespace() || ('\x1c'..='\x1f').contains(&character))
        {
            continue;
        }
        if !tail.is_empty() {
            tail.push('\n');
        }
        tail.push_str(text);
    }
    tail
}

pub fn read_skill_files(folder: &Path) -> Result<Vec<(String, Vec<u8>)>, PayloadError> {
    if !folder.join(SKILL_MANIFEST).is_file() {
        return Err(PayloadError::NoManifest(folder.to_path_buf()));
    }
    let root = fs::canonicalize(folder)
        .map_err(|source| PayloadError::Read(folder.to_path_buf(), source))?;
    let mut files = Vec::new();
    collect_skill_files(&root, &root, &mut files)?;
    Ok(files)
}

fn collect_skill_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), PayloadError> {
    let mut entries = fs::read_dir(directory)
        .and_then(|listing| {
            listing
                .map(|entry| entry.map(|found| found.path()))
                .collect::<io::Result<Vec<_>>>()
        })
        .map_err(|source| PayloadError::Read(directory.to_path_buf(), source))?;
    entries.sort();
    let mut folders = Vec::new();
    for path in entries {
        if fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
            // os.walk lists a linked folder but never enters it
            if !path.is_symlink() {
                folders.push(path);
            }
            continue;
        }
        let contents =
            fs::read(&path).map_err(|source| PayloadError::Read(path.clone(), source))?;
        let relative = path.strip_prefix(root).ok().and_then(Path::to_str);
        let name = relative.ok_or_else(|| PayloadError::PathNotUtf8(path.clone()))?;
        files.push((name.replace(MAIN_SEPARATOR, "/"), contents));
    }
    for folder in folders {
        collect_skill_files(root, &folder, files)?;
    }
    Ok(())
}
