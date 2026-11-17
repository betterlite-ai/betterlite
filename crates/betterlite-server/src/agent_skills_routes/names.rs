use std::collections::BTreeMap;

use betterlite_core::api::stored_skill_reads::stored_skill::StoredSkill;
use betterlite_http::skill_archive::SkillArchive;

// LiteLLM's MAX_SKILL_NAME_LENGTH, MAX_SKILL_DESCRIPTION_LENGTH and _FALLBACK_SKILL_NAME
const MAX_SKILL_NAME_LENGTH: usize = 64;
const MAX_SKILL_DESCRIPTION_LENGTH: usize = 1_024;
const FALLBACK_SKILL_NAME: &str = "skill";

pub(crate) fn base_name(skill: &StoredSkill, archive: &SkillArchive) -> String {
    if let Some(name) = archive.declared_name.as_deref().and_then(slugify) {
        return name;
    }
    if let Some(name) = skill.display_title.as_deref().and_then(slugify) {
        return name;
    }
    slugify(&skill.skill_id).unwrap_or_else(|| FALLBACK_SKILL_NAME.to_owned())
}

// LiteLLM's _deduplicated: a name's ordinal is its position inside its group, and a stable sort keeps the original order
pub(crate) fn deduplicate(names: &[String]) -> Vec<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    names
        .iter()
        .map(|name| {
            let ordinal = seen.entry(name.as_str()).or_insert(0);
            let position = *ordinal;
            *ordinal = position.saturating_add(1);
            with_ordinal(name, position)
        })
        .collect()
}

pub(crate) fn entry_description(skill: &StoredSkill, archive: &SkillArchive, name: &str) -> String {
    let chosen = [
        archive.declared_description.as_deref(),
        skill.description.as_deref(),
        skill.display_title.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find_map(|candidate| {
        let trimmed = candidate.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
    .unwrap_or(name);
    chosen.chars().take(MAX_SKILL_DESCRIPTION_LENGTH).collect()
}

// LiteLLM's _slugify: lowercased, every run outside [a-z0-9] becomes one -, both ends trimmed, cut to 64 and trimmed again
fn slugify(raw: &str) -> Option<String> {
    let mut slug = String::with_capacity(raw.len());
    for character in raw.to_lowercase().chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            slug.push(character);
            continue;
        }
        if slug.ends_with('-') {
            continue;
        }
        slug.push('-');
    }
    let cut: String = slug
        .trim_matches('-')
        .chars()
        .take(MAX_SKILL_NAME_LENGTH)
        .collect();
    let shortened = cut.trim_end_matches('-');
    if shortened.is_empty() {
        return None;
    }
    Some(shortened.to_owned())
}

// LiteLLM's _with_ordinal: the suffix's own length comes out of the 64 characters
fn with_ordinal(name: &str, ordinal: usize) -> String {
    if ordinal == 0 {
        return name.to_owned();
    }
    let next = ordinal.saturating_add(1);
    let suffix = format!("-{next}");
    let room = MAX_SKILL_NAME_LENGTH.saturating_sub(suffix.len());
    let stem: String = name.chars().take(room).collect();
    format!("{}{suffix}", stem.trim_end_matches('-'))
}
