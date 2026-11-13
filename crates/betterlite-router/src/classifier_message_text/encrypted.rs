use betterlite_models::decisions::Json;

use crate::classifier_message_text::asks::json_human_asks_newest_first;
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::MarkerPair;

const AGENT_MESSAGE_TYPE: &str = "agent_message";
const ENCRYPTED_CONTENT_TYPE: &str = "encrypted_content";
const KEPT_CONTENT_TYPES: [&str; 2] = ["input_text", "encrypted_content"];

pub fn encrypted_classifier_task<E, F>(
    input: Option<&Json>,
    has_messages: bool,
    mut resolve_item: F,
    markers: &[MarkerPair<'_>],
) -> Result<Option<Json>, E>
where
    E: From<ClassifierTextError>,
    F: FnMut(&Json) -> Result<Vec<Json>, E>,
{
    let Some(Json::List(items)) = input else {
        return Ok(None);
    };
    if has_messages {
        return Ok(None);
    }
    if items.iter().any(|item| !matches!(item, Json::Object(_))) {
        return Ok(None);
    }
    let mut current = None;
    for item in items.iter().rev() {
        let resolved = resolve_item(item)?;
        if !resolved.is_empty()
            && !json_human_asks_newest_first(&resolved, markers)
                .map_err(E::from)?
                .is_empty()
        {
            current = Some(item);
            break;
        }
    }
    let Some(current) = current else {
        return Ok(None);
    };
    let Json::Object(fields) = current else {
        return Ok(None);
    };
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == AGENT_MESSAGE_TYPE) {
        return Ok(None);
    }
    let Some(Json::List(parts)) = fields.get("content") else {
        return Ok(None);
    };
    if parts.iter().any(|part| !matches!(part, Json::Object(_))) {
        return Ok(None);
    }
    if !encrypts_content(parts)? {
        return Ok(None);
    }
    let kept: Vec<Json> = parts
        .iter()
        .filter(|part| match part {
            Json::Object(part_fields) => matches!(
                part_fields.get("type"),
                Some(Json::Text(kind)) if KEPT_CONTENT_TYPES.contains(&kind.as_str())
            ),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
        })
        .cloned()
        .collect();
    let mut output = fields.clone();
    output.insert("content".to_owned(), Json::List(kept));
    Ok(Some(Json::Object(output)))
}

fn encrypts_content(parts: &[Json]) -> Result<bool, ClassifierTextError> {
    for part in parts {
        let Json::Object(fields) = part else {
            continue;
        };
        if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == ENCRYPTED_CONTENT_TYPE) {
            continue;
        }
        match fields.get("encrypted_content") {
            Some(Json::Text(content)) if !content.is_empty() => return Ok(true),
            None | Some(Json::Null | Json::Text(_)) => {}
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(ClassifierTextError::EncryptedContentNotString);
            }
        }
    }
    Ok(false)
}
