use std::io::{Read as _, Seek as _, SeekFrom};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::api::image_input_options::mime::image_content_type_seekable;
use betterlite_openai::openai_video_identifiers::multipart::ReadSeek;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VeoRequestError;

// The injected B1120 reader: LiteLLM calls ImageEditRequestUtils.get_image_content_type
pub trait MimeReader {
    fn mime(&self, reader: &mut dyn ReadSeek) -> &'static str;
}

// B1120's seekable probe restores the position it found, so the conversion still seeks to zero
pub struct B1120Mime;

impl MimeReader for B1120Mime {
    fn mime(&self, mut reader: &mut dyn ReadSeek) -> &'static str {
        image_content_type_seekable(&mut reader)
    }
}

// The image arrives as a Vertex dict, a bare gs:// URI, or an open binary stream
pub enum ImageInput<'media> {
    Json(Json),
    File(&'media mut dyn ReadSeek),
}

pub fn image_object(
    image: ImageInput<'_>,
    mime: &dyn MimeReader,
) -> Result<Object, VeoRequestError> {
    match image {
        ImageInput::Json(Json::Object(fields)) => Ok(fields),
        ImageInput::Json(Json::Text(text)) if text.starts_with("gs://") => {
            let mut object = Object::new();
            object.insert("gcsUri".to_owned(), Json::Text(text));
            Ok(object)
        }
        ImageInput::Json(Json::Text(text)) => {
            Err(VeoRequestError::UnsupportedImage { value: text })
        }
        ImageInput::Json(_) => Err(VeoRequestError::ImageShape),
        ImageInput::File(reader) => encoded_image(reader, mime),
    }
}

// _convert_image_to_vertex_format: probe the type, then seek to zero and read the whole stream
fn encoded_image(
    reader: &mut dyn ReadSeek,
    mime: &dyn MimeReader,
) -> Result<Object, VeoRequestError> {
    let content_type = mime.mime(&mut *reader);
    reader
        .seek(SeekFrom::Start(0))
        .map_err(VeoRequestError::ImageRead)?;
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(VeoRequestError::ImageRead)?;
    let mut object = Object::new();
    object.insert(
        "bytesBase64Encoded".to_owned(),
        Json::Text(STANDARD.encode(bytes)),
    );
    object.insert("mimeType".to_owned(), Json::Text(content_type.to_owned()));
    Ok(object)
}
