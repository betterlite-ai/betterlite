use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::error::CanvasEditError;
use super::types::CanvasImageInput;

// B159 reads a file-like input, so this only encodes bytes and passes an already base64 string through.
pub fn file_types_to_b64(image: Option<&CanvasImageInput>) -> Result<String, CanvasEditError> {
    match image {
        None => Err(CanvasEditError::MissingImage),
        Some(CanvasImageInput::Bytes(content)) => Ok(BASE64.encode(content)),
        Some(CanvasImageInput::Text(text)) => Ok(text.clone()),
        Some(CanvasImageInput::Tuple) => Err(CanvasEditError::TupleFileType),
    }
}
