use sha2::{Digest as _, Sha256};

use crate::bedrock_images::formats::SUPPORTED_VIDEO_FORMATS;
use crate::bedrock_images::types::{
    ContentBlock, DocumentBlock, ImageBlock, SourceBlock, VideoBlock,
};

const HASH_SAMPLE_BYTES: usize = 64 * 1024;
const CONTENT_HASH_CHARS: usize = 16;

#[must_use]
pub fn create_bedrock_block(
    image_bytes: &str,
    mime_type: &str,
    image_format: &str,
) -> ContentBlock {
    let source = SourceBlock {
        bytes: image_bytes.to_owned(),
    };
    if mime_type.starts_with("application") || mime_type.starts_with("text") {
        return ContentBlock::Document {
            document: DocumentBlock {
                format: image_format.to_owned(),
                source,
                name: document_name(image_bytes, image_format),
            },
        };
    }
    if SUPPORTED_VIDEO_FORMATS
        .iter()
        .any(|video_format| image_format.starts_with(video_format))
    {
        return ContentBlock::Video {
            video: VideoBlock {
                format: image_format.to_owned(),
                source,
            },
        };
    }
    ContentBlock::Image {
        image: ImageBlock {
            format: image_format.to_owned(),
            source,
        },
    }
}

// LiteLLM hashes the first 64 KiB of the payload after Python's str.split(), whose whitespace set adds U+001C..U+001F to Rust's
fn document_name(image_bytes: &str, image_format: &str) -> String {
    let joined: String = image_bytes
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
        })
        .collect();
    let normalized = joined.as_bytes();
    let sample = normalized.get(..HASH_SAMPLE_BYTES).unwrap_or(normalized);
    let mut hasher = Sha256::new();
    hasher.update(sample);
    hasher.update(normalized.len().to_string().as_bytes());
    let digest = hex::encode(hasher.finalize());
    let content_hash = digest.get(..CONTENT_HASH_CHARS).unwrap_or(&digest);
    format!("DocumentPDFmessages_{content_hash}_{image_format}")
}
