use crate::arguments::json::{Json, Object};
use crate::file_ids::limits::{FileIdsError, check_messages};
use crate::file_ids::managed_id::{get_format_from_file_id, llm_output_file_id, original_file_id};
use crate::file_ids::mapping::FileIdMapping;
use crate::message_content::{Content, Message};

pub fn update_messages_with_file_ids(
    messages: &mut [Message],
    model_id: Option<&str>,
    mapping: &FileIdMapping,
) -> Result<(), FileIdsError> {
    check_messages(messages)?;
    for message in messages.iter_mut() {
        if message.role != "user" {
            continue;
        }
        let Some(Content::Parts(parts)) = &mut message.content else {
            continue;
        };
        for part in parts.iter_mut() {
            if part.kind != "file" {
                continue;
            }
            let Some(Json::Object(file)) = part.fields.get_mut("file") else {
                continue;
            };
            update_file(file, model_id, mapping);
        }
    }
    Ok(())
}

fn update_file(file: &mut Object, model_id: Option<&str>, mapping: &FileIdMapping) {
    let file_id = file
        .get("file_id")
        .and_then(Json::as_str)
        .map(str::to_owned);
    let derived_format = if file.contains_key("format") {
        None
    } else {
        get_format_from_file_id(file_id.as_deref())
    };
    if let Some(present) = file_id.as_deref().filter(|value| !value.is_empty()) {
        let provider = model_id
            .and_then(|model| mapping.provider_file_id(present, model))
            .map(str::to_owned)
            .or_else(|| llm_output_file_id(present).filter(|value| !value.is_empty()))
            .or_else(|| original_file_id(present));
        let value = provider.unwrap_or_else(|| present.to_owned());
        file.insert("file_id".to_owned(), Json::Text(value));
    }
    if let Some(format) = derived_format.filter(|value| !value.is_empty()) {
        file.insert("format".to_owned(), Json::Text(format));
    }
}

pub fn get_file_ids_from_messages(messages: &[Message]) -> Result<Vec<String>, FileIdsError> {
    check_messages(messages)?;
    let mut file_ids = Vec::new();
    for message in messages {
        if message.role != "user" {
            continue;
        }
        let Some(Content::Parts(parts)) = &message.content else {
            continue;
        };
        for part in parts {
            if part.kind != "file" {
                continue;
            }
            let Some(Json::Object(file)) = part.fields.get("file") else {
                continue;
            };
            if let Some(file_id) = file
                .get("file_id")
                .and_then(Json::as_str)
                .filter(|value| !value.is_empty())
            {
                file_ids.push(file_id.to_owned());
            }
        }
    }
    Ok(file_ids)
}
