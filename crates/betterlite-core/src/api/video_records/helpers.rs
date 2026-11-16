use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::encode_character_id;
use indexmap::IndexMap;

use super::{CharacterObject, VideoRecordError};

const ID_FIELD: &str = "id";

// LiteLLM mixes a dict and an object reply on the character routes, so the id encoder takes either.
#[derive(Debug)]
pub enum CharacterResponse<'response> {
    Object(&'response mut CharacterObject),
    Fields(&'response mut IndexMap<String, Json>),
}

// ast-grep-ignore: no-long-names
pub fn encode_character_id_in_response(
    response: CharacterResponse<'_>,
    custom_llm_provider: &str,
    model_id: Option<&str>,
) -> Result<(), VideoRecordError> {
    match response {
        CharacterResponse::Object(character) => {
            character.id = encode_character_id(&character.id, custom_llm_provider, model_id);
            Ok(())
        }
        CharacterResponse::Fields(fields) => {
            encode_fields_character_id(fields, custom_llm_provider, model_id)
        }
    }
}

// ast-grep-ignore: no-long-names
pub fn extract_model_from_target_model_names(
    target_model_names: &Json,
) -> Result<Option<String>, VideoRecordError> {
    match target_model_names {
        Json::Text(names) => Ok(names
            .split(',')
            .map(str::trim)
            .find(|name| !name.is_empty())
            .map(str::to_owned)),
        Json::List(names) => first_model(names),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(None),
    }
}

// None is LiteLLM's answer for an object whose id is an explicit null: get("id", "") hands the null back.
pub fn video_reference_to_id(video_reference: &Json) -> Result<Option<String>, VideoRecordError> {
    match video_reference {
        Json::Object(fields) => id_of(fields),
        Json::Text(text) => {
            let Ok(parsed) = serde_json::from_str::<Json>(text) else {
                return Ok(Some(text.clone()));
            };
            match parsed {
                Json::Object(fields) => id_of(&fields),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    Ok(Some(text.clone()))
                }
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Ok(Some(String::new())),
    }
}

pub fn get_custom_provider_from_data(
    data: &IndexMap<String, Json>,
) -> Result<Option<String>, VideoRecordError> {
    match data.get("custom_llm_provider") {
        Some(Json::Null) | None => {}
        Some(Json::Text(provider)) if provider.is_empty() => {}
        Some(Json::Text(provider)) => return Ok(Some(provider.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(VideoRecordError::NotText {
                field: "custom_llm_provider",
            });
        }
    }
    match data.get("extra_body") {
        Some(Json::Text(text)) => Ok(provider_from_extra_body(text)),
        Some(Json::Object(fields)) => Ok(extra_body_provider(fields)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => Ok(None),
    }
}

fn first_model(names: &[Json]) -> Result<Option<String>, VideoRecordError> {
    match names.first() {
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VideoRecordError::NotText {
                field: "target_model_names",
            })
        }
    }
}

fn id_of(fields: &IndexMap<String, Json>) -> Result<Option<String>, VideoRecordError> {
    match fields.get(ID_FIELD) {
        Some(Json::Text(id)) => Ok(Some(id.clone())),
        Some(Json::Null) => Ok(None),
        None => Ok(Some(String::new())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VideoRecordError::NotText { field: ID_FIELD })
        }
    }
}

fn provider_from_extra_body(text: &str) -> Option<String> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(fields)) => extra_body_provider(&fields),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => None,
    }
}

// LiteLLM keeps a non-string provider inside extra_body as no provider, so this path returns None.
fn extra_body_provider(fields: &IndexMap<String, Json>) -> Option<String> {
    match fields.get("custom_llm_provider") {
        Some(Json::Text(provider)) => Some(provider.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn encode_fields_character_id(
    fields: &mut IndexMap<String, Json>,
    custom_llm_provider: &str,
    model_id: Option<&str>,
) -> Result<(), VideoRecordError> {
    match fields.get(ID_FIELD) {
        None | Some(Json::Null) => Ok(()),
        Some(Json::Text(character_id)) => {
            let encoded = encode_character_id(character_id, custom_llm_provider, model_id);
            fields.insert(ID_FIELD.to_owned(), Json::Text(encoded));
            Ok(())
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VideoRecordError::NotText { field: ID_FIELD })
        }
    }
}
