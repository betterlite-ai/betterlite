use std::collections::BTreeSet;
use std::io::{Cursor, Read as _, Write as _};

use regex::Regex;
use serde::Deserialize;
use serde::de::IgnoredAny;
use sha2::{Digest as _, Sha256};
use zip::result::{ZipError, ZipResult};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

const MAX_ARCHIVE_UNPACKED_BYTES: u64 = 50 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 1_000;
const SKILL_MANIFEST_FILENAME: &str = "SKILL.md";
const FRONTMATTER_PATTERN: &str = r"(?s)^---\s*\n(.*?)\n---\s*(?:\n|$)";

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
    uploaded.validate_header_consistency().ok()?;
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
        let raw_name = uploaded.name_for_index(index)?.ok()?.into_owned();
        if raw_name.ends_with('/') {
            continue;
        }
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return None;
        }
        let path = normalize_path(&raw_name)?;
        let mut info = uploaded.by_index(index).ok()?;
        unpacked_bytes = unpacked_bytes.checked_add(info.size())?;
        if unpacked_bytes > MAX_ARCHIVE_UNPACKED_BYTES {
            return None;
        }
        let mut data = Vec::new();
        info.read_to_end(&mut data).ok()?;
        entries.push((path, data));
    }
    if entries.is_empty() {
        return None;
    }
    let path_list: Vec<&str> = entries.iter().map(|(path, _)| path.as_str()).collect();
    let prefix = common_root_prefix(&path_list);
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

fn common_root_prefix(path_list: &[&str]) -> String {
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
    #[serde(default)]
    name: Option<FrontmatterField>,
    #[serde(default)]
    description: Option<FrontmatterField>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FrontmatterField {
    Text(String),
    Other(IgnoredAny),
}

fn manifest_frontmatter(manifest: &[u8]) -> (Option<String>, Option<String>) {
    #[expect(
        clippy::disallowed_methods,
        reason = "LiteLLM decodes SKILL.md with errors=replace"
    )]
    let text = String::from_utf8_lossy(manifest).into_owned();
    let Ok(pattern) = Regex::new(FRONTMATTER_PATTERN) else {
        return (None, None);
    };
    let Some(captures) = pattern.captures(&text) else {
        return (None, None);
    };
    let Some(yaml) = captures.get(1) else {
        return (None, None);
    };
    let options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins,
    };
    let Ok(frontmatter) =
        serde_saphyr::from_str_with_options::<Frontmatter>(yaml.as_str(), options)
    else {
        return (None, None);
    };
    (
        frontmatter_text(frontmatter.name),
        frontmatter_text(frontmatter.description),
    )
}

fn frontmatter_text(field: Option<FrontmatterField>) -> Option<String> {
    match field? {
        FrontmatterField::Text(text) => clean_frontmatter_text(&text),
        FrontmatterField::Other(_) => None,
    }
}

fn clean_frontmatter_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn repack(members: &[(String, Vec<u8>)]) -> ZipResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::DEFAULT)
        .external_attributes(0o644 << 16);
    for (name, data) in members {
        writer.start_file(name, options)?;
        writer.write_all(data).map_err(ZipError::Io)?;
    }
    writer.finish()?;
    Ok(output.into_inner())
}

mod tests;
