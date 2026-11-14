use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

const AVAILABLE_SKILLS_HEADER: &str = "\n\n---\n\n# Available Skills\n\n";
const SKILL_SEPARATOR: &str = "\n\n---\n\n";

#[derive(Debug, Error)]
pub enum SkillContentError {
    #[error("messages must be a list")]
    MessagesNotList,
    #[error("system must be a string")]
    SystemNotText,
    #[error("the first system message content must be a string")]
    SystemContentNotText,
}

// ast-grep-ignore: no-long-names
pub fn inject_skill_content_to_messages(
    data: &mut Object,
    skill_contents: &[String],
    use_anthropic_format: bool,
) -> Result<(), SkillContentError> {
    if skill_contents.is_empty() {
        return Ok(());
    }
    let section = format!(
        "{AVAILABLE_SKILLS_HEADER}{}",
        skill_contents.join(SKILL_SEPARATOR)
    );
    if use_anthropic_format {
        let absent = section.trim();
        return append_section(
            data,
            "system",
            &section,
            absent,
            SkillContentError::SystemNotText,
        );
    }
    inject_messages(data, &section)
}

fn inject_messages(data: &mut Object, section: &str) -> Result<(), SkillContentError> {
    let messages = match data.get_mut("messages") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::List(messages)) => messages,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(SkillContentError::MessagesNotList);
        }
    };
    if messages.is_empty() {
        return Ok(());
    }
    for message in &mut *messages {
        let Json::Object(fields) = message else {
            continue;
        };
        if fields.get("role").and_then(Json::as_str) != Some("system") {
            continue;
        }
        return append_section(
            fields,
            "content",
            section,
            section,
            SkillContentError::SystemContentNotText,
        );
    }
    messages.insert(
        0,
        Json::Object(Object::from([
            ("role".to_owned(), Json::Text("system".to_owned())),
            ("content".to_owned(), Json::Text(section.trim().to_owned())),
        ])),
    );
    Ok(())
}

fn append_section(
    fields: &mut Object,
    key: &str,
    section: &str,
    absent: &str,
    missing: SkillContentError,
) -> Result<(), SkillContentError> {
    let value = match fields.get(key) {
        None | Some(Json::Null) => Json::Text(absent.to_owned()),
        Some(Json::Text(text)) => Json::Text(format!("{text}{section}")),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(missing);
        }
    };
    fields.insert(key.to_owned(), value);
    Ok(())
}
