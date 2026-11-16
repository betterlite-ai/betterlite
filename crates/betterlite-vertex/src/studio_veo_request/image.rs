use betterlite_prompts::arguments::json::{Json, Object};

use super::error::StudioVeoError;
use crate::vertex_veo_request::image::{ImageInput, MimeReader, image_object};

// transform_video_create_request: a dictionary stays as it is, and anything without a file reader cannot convert
pub fn gemini_image(
    image: ImageInput<'_>,
    mime: &dyn MimeReader,
) -> Result<Object, StudioVeoError> {
    match image {
        ImageInput::Json(Json::Object(fields)) => Ok(fields),
        ImageInput::Json(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        ) => Err(StudioVeoError::ImageNotAFile),
        ImageInput::File(reader) => {
            image_object(ImageInput::File(reader), mime).map_err(StudioVeoError::ImageRead)
        }
    }
}
