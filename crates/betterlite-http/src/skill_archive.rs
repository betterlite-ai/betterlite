use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::{Cursor, Read as _, Write as _};
use std::str;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use zip::result::{ZipError, ZipResult};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

const MAX_ARCHIVE_UNPACKED_BYTES: u64 = 50 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 1_000;
const SKILL_MANIFEST_FILENAME: &str = "SKILL.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillArchive {
    pub content: Vec<u8>,
    pub digest: String,
    pub declared_name: Option<String>,
    pub declared_description: Option<String>,
}

#[must_use]
pub fn build_skill_archive(stored_content: &[u8]) -> Option<SkillArchive> {
    let mut uploaded = ZipArchive::new(Cursor::new(stored_content)).ok()?;
    let members = flattened_members(&mut uploaded)?;
    let manifest = members
        .iter()
        .find_map(|(name, data)| (name == SKILL_MANIFEST_FILENAME).then_some(data.as_slice()))?;
    let (declared_name, declared_description) = manifest_frontmatter(manifest);
    let content = repack(&members).ok()?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&content)));
    Some(SkillArchive {
        content,
        digest,
        declared_name,
        declared_description,
    })
}

fn flattened_members(uploaded: &mut ZipArchive<Cursor<&[u8]>>) -> Option<Vec<(String, Vec<u8>)>> {
    let mut entries = Vec::new();
    let mut unpacked_bytes = 0_u64;
    for index in 0..uploaded.len() {
        let mut info = uploaded.by_index(index).ok()?;
        if info.is_dir() {
            continue;
        }
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return None;
        }
        unpacked_bytes = unpacked_bytes.checked_add(info.size())?;
        if unpacked_bytes > MAX_ARCHIVE_UNPACKED_BYTES {
            return None;
        }
        let raw_name = info.name().ok()?;
        let path = normalize_path(raw_name.as_ref())?;
        let mut data = Vec::new();
        info.read_to_end(&mut data).ok()?;
        entries.push((path, data));
    }
    if entries.is_empty() {
        return None;
    }
    let prefix = common_root_prefix(entries.iter().map(|(path, _)| path.as_str()));
    let mut names = BTreeSet::new();
    let mut flattened = Vec::with_capacity(entries.len());
    for (path, data) in entries {
        let name = path.strip_prefix(&prefix)?.to_owned();
        if !names.insert(name.clone()) {
            return None;
        }
        flattened.push((name, data));
    }
    if !names.contains(SKILL_MANIFEST_FILENAME) {
        return None;
    }
    flattened.sort_by(|left, right| left.0.cmp(&right.0));
    Some(flattened)
}

fn normalize_path(raw_path: &str) -> Option<String> {
    let bytes = raw_path.as_bytes();
    let drive_letter = bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1).is_some_and(|byte| *byte == b':');
    if raw_path.is_empty()
        || raw_path.contains('\0')
        || raw_path.contains('\\')
        || raw_path.starts_with('/')
        || drive_letter
    {
        return None;
    }
    let parts: Vec<&str> = raw_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() || parts.iter().any(|part| matches!(*part, "." | "..")) {
        return None;
    }
    Some(parts.join("/"))
}

fn common_root_prefix<'path>(path_iter: impl Iterator<Item = &'path str>) -> String {
    let path_list: Vec<&str> = path_iter.collect();
    let Some(first) = path_list.first() else {
        return String::new();
    };
    let Some((root, _)) = first.split_once('/') else {
        return String::new();
    };
    if path_list.iter().all(|path| {
        path.split_once('/')
            .is_some_and(|(candidate, _)| candidate == root)
    }) {
        format!("{root}/")
    } else {
        String::new()
    }
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

fn manifest_frontmatter(manifest: &[u8]) -> (Option<String>, Option<String>) {
    let text = decode_utf8_replacing(manifest);
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return (None, None);
    };
    if first.trim_end_matches(['\r', '\n']).trim() != "---" {
        return (None, None);
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']).trim() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
    }
    if !closed {
        return (None, None);
    }
    let Ok(frontmatter) = serde_saphyr::from_str::<Frontmatter>(&yaml) else {
        return (None, None);
    };
    (
        clean_frontmatter_text(frontmatter.name),
        clean_frontmatter_text(frontmatter.description),
    )
}

fn decode_utf8_replacing(input: &[u8]) -> String {
    let mut remaining = input;
    let mut output = String::new();
    while !remaining.is_empty() {
        match str::from_utf8(remaining) {
            Ok(valid) => {
                output.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_length = error.valid_up_to();
                if let Some(valid) = remaining.get(..valid_length) {
                    output.push_str(str::from_utf8(valid).unwrap_or_default());
                }
                output.push('\u{FFFD}');
                let invalid_length = error
                    .error_len()
                    .unwrap_or_else(|| remaining.len().saturating_sub(valid_length));
                remaining = remaining
                    .get(valid_length.saturating_add(invalid_length)..)
                    .unwrap_or_default();
            }
        }
    }
    output
}

fn clean_frontmatter_text(value: Option<String>) -> Option<String> {
    let text = value?;
    let trimmed = text.trim().to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn repack(members: &[(String, Vec<u8>)]) -> ZipResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
                .map_err(|error| ZipError::InvalidArchive(Cow::Owned(error.to_string())))?,
        )
        .unix_permissions(0o644);
    for (name, data) in members {
        writer.start_file(name, options)?;
        writer.write_all(data).map_err(ZipError::Io)?;
    }
    writer.finish()?;
    Ok(output.into_inner())
}

mod tests;
