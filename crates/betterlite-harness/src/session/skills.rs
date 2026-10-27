use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::payloads::SKILL_MANIFEST;

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("skill base directory must be absolute")]
    RelativeDirectory,
    #[error("Skill folder {} has no {SKILL_MANIFEST}", .0.display())]
    NoManifest(PathBuf),
    #[error("skill path {} is not UTF-8", .0.display())]
    PathNotUtf8(PathBuf),
}

pub fn normalize_skill(skill: &Path, directory: &Path) -> Result<String, SkillError> {
    if !directory.is_absolute() {
        return Err(SkillError::RelativeDirectory);
    }
    let absolute = directory.join(skill);
    let mut normalized = PathBuf::new();
    // abspath normalizes lexically; canonicalize would change symlink identity
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component);
            }
        }
    }
    if !normalized.join(SKILL_MANIFEST).is_file() {
        return Err(SkillError::NoManifest(normalized));
    }
    normalized
        .to_str()
        .map(str::to_owned)
        .ok_or(SkillError::PathNotUtf8(normalized))
}
