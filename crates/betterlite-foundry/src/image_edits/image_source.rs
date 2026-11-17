use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::error::ImageEditError;

// Only a BufferedReader is rewound; any other reader is consumed, and a tuple or URL string is one image value, never a list of references or a fetch
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    Bytes(Vec<u8>),
    Buffered { content: Vec<u8>, position: usize },
    Reader { content: Vec<u8>, position: usize },
    Unsupported { kind: String },
}

pub fn convert_image_to_base64(image: &mut ImageSource) -> Result<String, ImageEditError> {
    let bytes = match image {
        ImageSource::Bytes(content) => content.as_slice(),
        ImageSource::Buffered { content, position } => {
            let tail = content.get(*position..).unwrap_or_default();
            *position = 0;
            tail
        }
        ImageSource::Reader { content, position } => {
            let tail = content.get(*position..).unwrap_or_default();
            *position = content.len();
            tail
        }
        ImageSource::Unsupported { kind } => {
            return Err(ImageEditError::UnsupportedImageType { kind: kind.clone() });
        }
    };
    Ok(BASE64.encode(bytes))
}
