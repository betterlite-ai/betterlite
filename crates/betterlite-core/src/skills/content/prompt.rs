use betterlite_db::managed_resource_rows::skills::SkillTable;

use super::decode::{DecodeError, open_skill_archive};

const SKILL_SUFFIXES: [&str; 3] = ["SKILL.md", "README.md", ".md"];

#[must_use]
pub fn extract_skill_content(skill: &SkillTable) -> Option<String> {
    let found = markdown_content(skill);
    if let Err(error) = &found {
        tracing::warn!(
            "SkillPromptInjectionHandler: Error extracting content from skill {}: {error}",
            skill.skill_id
        );
    }
    match found {
        Ok(Some(text)) => {
            let title = skill
                .display_title
                .as_deref()
                .unwrap_or(skill.skill_id.as_str());
            Some(format!("## Skill: {title}\n\n{text}"))
        }
        Ok(None) | Err(_) => skill.instructions.clone(),
    }
}

fn markdown_content(skill: &SkillTable) -> Result<Option<String>, DecodeError> {
    let Some(mut archive) = open_skill_archive(skill)? else {
        return Ok(None);
    };
    let names = archive.entry_names()?;
    for suffix in SKILL_SUFFIXES {
        for (index, name) in names.iter().enumerate() {
            if !name.ends_with(suffix) {
                continue;
            }
            let text = archive.read_text(index, name)?;
            if !text.is_empty() {
                return Ok(Some(text));
            }
        }
    }
    Ok(None)
}
