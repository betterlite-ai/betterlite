use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::error::ImageEditError;

// _convert_image_to_base64 tells a BufferedReader apart from every other read-capable object, and only that one is
// rewound; a tuple or a URL string is one such value, not a list of references and never a fetch
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Bytes(Vec<u8>),
    Buffered { content: Vec<u8>, position: usize },
    Reader { content: Vec<u8>, position: usize },
    Unsupported { kind: String },
}

pub fn convert_image_to_base64(image: &mut ImageSource) -> Result<String, ImageEditError> {
    let bytes = match image {
        Self::Bytes(content) => content.as_slice(),
        Self::Buffered { content, position } => {
            let tail = content.get(*position..).unwrap_or_default();
            *position = 0;
            tail
        }
        Self::Reader { content, position } => content.get(*position..).unwrap_or_default(),
        Self::Unsupported { kind } => {
            return Err(ImageEditError::UnsupportedImageType { kind: kind.clone() });
        }
    };
    Ok(BASE64.encode(bytes))
}
