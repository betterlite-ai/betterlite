use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::openai_video_identifiers::multipart::MultipartPart as MediaMultipartPart;

use super::{MediaPart, MediaPartBody};

// B512's multipart_parts output, which already carries the edit's field names and repeated image parts
pub(super) fn http_part(part: MultipartPart) -> MediaPart {
    let MultipartPart {
        name,
        file_name,
        content_type,
        data,
    } = part;
    MediaPart {
        field: name,
        filename: file_name,
        content_type,
        body: MediaPartBody::Bytes(data),
    }
}

// B525 and B526 build their parts on MediaBody; an upload held in memory is copied into the record
pub(super) fn media_part(part: MediaMultipartPart<'_>) -> MediaPart {
    let MediaMultipartPart {
        field,
        filename,
        content_type,
        body,
    } = part;
    let part_body = match body {
        MediaBody::Bytes(bytes) => MediaPartBody::Bytes(bytes.to_vec()),
        MediaBody::Text(text) => MediaPartBody::Text(text.to_owned()),
        MediaBody::Stream { .. } => MediaPartBody::Streamed,
    };
    MediaPart {
        field,
        filename: Some(filename),
        content_type: Some(content_type.to_owned()),
        body: part_body,
    }
}
