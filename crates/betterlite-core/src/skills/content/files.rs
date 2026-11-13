use betterlite_db::managed_resource_rows::skills::SkillTable;

use crate::api::skills::hook::execution::SkillFiles;

use super::decode::{DecodeError, SkillArchive, open_skill_archive};

#[must_use]
pub fn extract_all_files(skill: &SkillTable) -> SkillFiles {
    let mut files = SkillFiles::new();
    let archive = match open_skill_archive(skill) {
        Ok(Some(archive)) => archive,
        Ok(None) => return files,
        Err(error) => {
            warn_files(&skill.skill_id, &error);
            return files;
        }
    };
    collect_entries(archive, skill, &mut files);
    files
}

fn collect_entries(mut archive: SkillArchive, skill: &SkillTable, files: &mut SkillFiles) {
    let names = match archive.entry_names() {
        Ok(names) => names,
        Err(error) => {
            warn_files(&skill.skill_id, &error);
            return;
        }
    };
    for (index, name) in names.iter().enumerate() {
        match entry_path(name) {
            EntryPath::Skip => {}
            EntryPath::Invalid(raw) => tracing::warn!(
                "SkillPromptInjectionHandler: Skipping entry with invalid path in skill {}: {raw}",
                skill.skill_id
            ),
            EntryPath::Accept(path) => {
                let Some(data) = read_entry(&mut archive, index, name, skill) else {
                    return;
                };
                if files.insert(path, data).is_some() {
                    tracing::warn!(
                        "SkillPromptInjectionHandler: Duplicate path in skill {} for entry {name}",
                        skill.skill_id
                    );
                }
            }
        }
    }
}

fn read_entry(
    archive: &mut SkillArchive,
    index: usize,
    name: &str,
    skill: &SkillTable,
) -> Option<Vec<u8>> {
    match archive.read_bytes(index, name) {
        Ok(data) => Some(data),
        Err(error) => {
            warn_files(&skill.skill_id, &error);
            None
        }
    }
}

fn warn_files(skill_id: &str, error: &DecodeError) {
    tracing::warn!(
        "SkillPromptInjectionHandler: Error extracting files from skill {skill_id}: {error}"
    );
}

enum EntryPath {
    Skip,
    Invalid(String),
    Accept(String),
}

fn entry_path(name: &str) -> EntryPath {
    if name.ends_with('/') {
        return EntryPath::Skip;
    }
    let clean = name.split_once('/').map_or(name, |(_, rest)| rest);
    if clean.is_empty() {
        return EntryPath::Skip;
    }
    let normalized = normalize_path(clean);
    if normalized.starts_with("..") || normalized.starts_with('/') {
        return EntryPath::Invalid(name.to_owned());
    }
    EntryPath::Accept(normalized)
}

// posixpath.normpath, including its reading of "..notes" as a parent reference
fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." if (!absolute && components.is_empty()) || components.last() == Some(&"..") => {
                components.push("..");
            }
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    let mut normalized = components.join("/");
    if absolute {
        normalized.insert(0, '/');
    }
    if normalized.is_empty() {
        return ".".to_owned();
    }
    normalized
}
