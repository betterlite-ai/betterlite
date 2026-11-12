use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;

// Longer inputs stay plain text, so classifying request data never decodes without a bound
pub const MEDIA_TEXT_LIMIT: usize = 0x0200_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Base64Image,
    GcsImage,
    GcsVideo,
}

#[must_use]
pub fn classify_embedding_media(input: &str) -> Option<MediaKind> {
    if input.contains("gs://") {
        return Some(if input.contains("mp4") {
            MediaKind::GcsVideo
        } else {
            MediaKind::GcsImage
        });
    }
    (input.len() <= MEDIA_TEXT_LIMIT && is_data_base64(input)).then_some(MediaKind::Base64Image)
}

#[must_use]
pub fn process_element(input: &str) -> Json {
    classify_embedding_media(input).map_or_else(
        || wrap("text", Json::Text(input.to_owned())),
        |kind| {
            let (name, payload) = media_fields(input, kind);
            wrap(name, payload)
        },
    )
}

#[must_use]
pub fn merged_instance(text: &str, media: &str, kind: MediaKind) -> Json {
    let (name, payload) = media_fields(media, kind);
    let mut instance = Extra::new();
    instance.insert(String::from("text"), Json::Text(text.to_owned()));
    instance.insert(name.to_owned(), payload);
    Json::Object(instance)
}

fn wrap(name: &str, payload: Json) -> Json {
    let mut instance = Extra::new();
    instance.insert(name.to_owned(), payload);
    Json::Object(instance)
}

fn media_fields(media: &str, kind: MediaKind) -> (&'static str, Json) {
    match kind {
        MediaKind::Base64Image => ("image", Json::Object(base64_fields(media))),
        MediaKind::GcsImage => ("image", Json::Object(gcs_fields(media))),
        MediaKind::GcsVideo => ("video", Json::Object(gcs_fields(media))),
    }
}

fn gcs_fields(uri: &str) -> Extra {
    let mut fields = Extra::new();
    fields.insert(String::from("gcsUri"), Json::Text(uri.to_owned()));
    fields
}

fn base64_fields(input: &str) -> Extra {
    let payload = input.split(',').nth(1).unwrap_or(input);
    let mut fields = Extra::new();
    fields.insert(
        String::from("bytesBase64Encoded"),
        Json::Text(payload.to_owned()),
    );
    fields
}

// Python's is_base64_encoded: a data: prefix, the second comma segment, strict decoding and a canonical re-encoding
fn is_data_base64(input: &str) -> bool {
    if !input.starts_with("data:") {
        return false;
    }
    let Some(payload) = input.split(',').nth(1) else {
        return false;
    };
    let Ok(decoded) = STANDARD.decode(payload) else {
        return false;
    };
    STANDARD.encode(decoded) == payload
}
