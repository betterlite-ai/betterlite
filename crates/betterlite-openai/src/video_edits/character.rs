use betterlite_http::url_path::encode_segment;

use super::VIDEO_FIELD;
use super::error::VideoEditError;
use crate::openai_video_identifiers::ids::decode_character_id;
use crate::openai_video_identifiers::multipart::{MediaBody, MultipartPart, video_file_tuple};

pub const CHARACTER_NAME_FIELD: &str = "name";

pub enum CharacterPart<'media> {
    Text {
        field: &'static str,
        text: &'media str,
    },
    File(MultipartPart<'media>),
}

pub struct CreateCharacter<'media> {
    pub url: String,
    pub files: Vec<CharacterPart<'media>>,
}

#[must_use]
pub fn transform_video_create_character_request<'media>(
    name: &'media str,
    video: MediaBody<'media>,
    api_base: &str,
) -> CreateCharacter<'media> {
    CreateCharacter {
        url: format!("{}/characters", api_base.trim_end_matches('/')),
        files: vec![
            CharacterPart::Text {
                field: CHARACTER_NAME_FIELD,
                text: name,
            },
            CharacterPart::File(video_file_tuple(video, VIDEO_FIELD)),
        ],
    }
}

pub fn transform_video_get_character_request(
    character_id: &str,
    api_base: &str,
) -> Result<String, VideoEditError> {
    let original = decode_character_id(character_id).character_id;
    let encoded = encode_segment(&original, "character_id")?;
    Ok(format!(
        "{}/characters/{encoded}",
        api_base.trim_end_matches('/')
    ))
}
