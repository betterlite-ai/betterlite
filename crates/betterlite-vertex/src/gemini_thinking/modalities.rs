use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseModality {
    Text,
    Image,
    Audio,
    Video,
    #[serde(rename = "MODALITY_UNSPECIFIED")]
    Unspecified,
}

#[must_use]
pub fn map_response_modalities(modalities: &[String]) -> Vec<ResponseModality> {
    modalities
        .iter()
        .map(|modality| match modality.as_str() {
            "text" => ResponseModality::Text,
            "image" => ResponseModality::Image,
            "audio" => ResponseModality::Audio,
            "video" => ResponseModality::Video,
            _ => ResponseModality::Unspecified,
        })
        .collect()
}
