use std::collections::BTreeSet;
use std::io::{Cursor, Read as _, Write as _};

use sha2::{Digest as _, Sha256};
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
        let path = normalize_path(info.name())?;
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
    if raw_path.is_empty()
        || raw_path.contains('\0')
        || raw_path.contains('\\')
        || raw_path.starts_with('/')
        || raw_path
            .get(..2)
            .is_some_and(|prefix| prefix.as_bytes()[1] == b':')
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

fn common_root_prefix<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let paths: Vec<&str> = paths.collect();
    let Some(first) = paths.first() else {
        return String::new();
    };
    let Some((root, _)) = first.split_once('/') else {
        return String::new();
    };
    if paths.iter().all(|path| {
        path.split_once('/')
            .is_some_and(|(candidate, _)| candidate == root)
    }) {
        format!("{root}/")
    } else {
        String::new()
    }
}

fn manifest_frontmatter(manifest: &[u8]) -> (Option<String>, Option<String>) {
    let text = String::from_utf8_lossy(manifest);
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
    let Ok(value) = serde_saphyr::from_str::<serde_json::Value>(&yaml) else {
        return (None, None);
    };
    let Some(object) = value.as_object() else {
        return (None, None);
    };
    (
        frontmatter_text(object.get("name")),
        frontmatter_text(object.get("description")),
    )
}

fn frontmatter_text(value: Option<&serde_json::Value>) -> Option<String> {
    let text = value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    text
}

fn repack(members: &[(String, Vec<u8>)]) -> zip::result::ZipResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
                .map_err(|error| zip::result::ZipError::InvalidArchive(error.to_string().into()))?,
        )
        .unix_permissions(0o644);
    for (name, data) in members {
        writer.start_file(name, options)?;
        writer.write_all(data).map_err(zip::result::ZipError::Io)?;
    }
    writer.finish()?;
    Ok(output.into_inner())
}

mod tests;
