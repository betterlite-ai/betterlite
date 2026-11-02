use std::collections::BTreeSet;
use std::io::{Cursor, Read as _, Write as _};
use std::ops::Range;
use std::str;

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
const LOCAL_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
const LOCAL_HEADER_FIXED_LEN: usize = 30;
const LOCAL_HEADER_NAME_LEN_RANGE: Range<usize> = 26..28;
const CENTRAL_HEADER_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const CENTRAL_HEADER_FIXED_LEN: usize = 46;
const CENTRAL_HEADER_FLAGS_RANGE: Range<usize> = 8..10;
const CENTRAL_HEADER_NAME_LEN_RANGE: Range<usize> = 28..30;
const LANGUAGE_ENCODING_FLAG: u16 = 0x0800;
const CP437_HIGH_BYTES: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00a2}', '\u{00a3}', '\u{00a5}', '\u{20a7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{2310}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{03b1}', '\u{00df}', '\u{0393}', '\u{03c0}', '\u{03a3}', '\u{03c3}', '\u{00b5}', '\u{03c4}',
    '\u{03a6}', '\u{0398}', '\u{03a9}', '\u{03b4}', '\u{221e}', '\u{03c6}', '\u{03b5}', '\u{2229}',
    '\u{2261}', '\u{00b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00f7}', '\u{2248}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{207f}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

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
    let members = flattened_members(stored_content, &mut uploaded)?;
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

fn flattened_members(
    stored_content: &[u8],
    uploaded: &mut ZipArchive<Cursor<&[u8]>>,
) -> Option<Vec<(String, Vec<u8>)>> {
    if uploaded.has_duplicate_names() {
        return None;
    }
    let mut entries = Vec::new();
    let mut unpacked_bytes = 0_u64;
    for index in 0..uploaded.len() {
        let member = read_validated_member(stored_content, uploaded, index, &mut unpacked_bytes)?;
        let ArchiveMember::File(path, data) = member else {
            continue;
        };
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return None;
        }
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

enum ArchiveMember {
    Directory,
    File(String, Vec<u8>),
}

fn read_validated_member(
    stored_content: &[u8],
    uploaded: &mut ZipArchive<Cursor<&[u8]>>,
    index: usize,
    unpacked_bytes: &mut u64,
) -> Option<ArchiveMember> {
    let mut info = uploaded.by_index(index).ok()?;
    let (central_name_raw, flags) =
        central_header_name_and_flags(stored_content, info.central_header_start())?;
    let name = decode_central_name(central_name_raw, flags & LANGUAGE_ENCODING_FLAG != 0)?;
    if name.ends_with('/') {
        return Some(ArchiveMember::Directory);
    }
    if !local_header_name_matches(stored_content, info.header_start(), central_name_raw)? {
        return None;
    }
    let path = normalize_path(&name)?;
    *unpacked_bytes = unpacked_bytes.checked_add(info.size())?;
    if *unpacked_bytes > MAX_ARCHIVE_UNPACKED_BYTES {
        return None;
    }
    let mut data = Vec::new();
    info.read_to_end(&mut data).ok()?;
    Some(ArchiveMember::File(path, data))
}

fn central_header_name_and_flags(
    stored_content: &[u8],
    central_header_start: u64,
) -> Option<(&[u8], u16)> {
    let start = usize::try_from(central_header_start).ok()?;
    let fixed = stored_content.get(start..start.checked_add(CENTRAL_HEADER_FIXED_LEN)?)?;
    let signature: [u8; 4] = fixed.get(..4)?.try_into().ok()?;
    if signature != CENTRAL_HEADER_SIGNATURE {
        return None;
    }
    let flags_bytes: [u8; 2] = fixed.get(CENTRAL_HEADER_FLAGS_RANGE)?.try_into().ok()?;
    let name_length_bytes: [u8; 2] = fixed.get(CENTRAL_HEADER_NAME_LEN_RANGE)?.try_into().ok()?;
    let name_length = usize::from(u16::from_le_bytes(name_length_bytes));
    let name_start = start.checked_add(CENTRAL_HEADER_FIXED_LEN)?;
    let name_raw = stored_content.get(name_start..name_start.checked_add(name_length)?)?;
    Some((name_raw, u16::from_le_bytes(flags_bytes)))
}

fn decode_central_name(raw: &[u8], is_utf8: bool) -> Option<String> {
    if is_utf8 {
        str::from_utf8(raw).ok().map(ToOwned::to_owned)
    } else {
        Some(
            raw.iter()
                .map(|&byte| {
                    usize::from(byte).checked_sub(0x80).map_or_else(
                        || char::from(byte),
                        |index| {
                            CP437_HIGH_BYTES
                                .get(index)
                                .copied()
                                .unwrap_or(char::REPLACEMENT_CHARACTER)
                        },
                    )
                })
                .collect(),
        )
    }
}

fn local_header_name_matches(
    stored_content: &[u8],
    header_start: u64,
    central_name_raw: &[u8],
) -> Option<bool> {
    let start = usize::try_from(header_start).ok()?;
    let fixed = stored_content.get(start..start.checked_add(LOCAL_HEADER_FIXED_LEN)?)?;
    let signature: [u8; 4] = fixed.get(..4)?.try_into().ok()?;
    if signature != LOCAL_HEADER_SIGNATURE {
        return None;
    }
    let name_length_bytes: [u8; 2] = fixed.get(LOCAL_HEADER_NAME_LEN_RANGE)?.try_into().ok()?;
    let name_length = usize::from(u16::from_le_bytes(name_length_bytes));
    let name_start = start.checked_add(LOCAL_HEADER_FIXED_LEN)?;
    let local_name_raw = stored_content.get(name_start..name_start.checked_add(name_length)?)?;
    Some(local_name_raw == central_name_raw)
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
    #[expect(clippy::disallowed_methods, reason = "LiteLLM uses errors=replace")]
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
