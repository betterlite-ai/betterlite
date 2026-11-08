use betterlite_vertex::model_urls::{ApiVersion, UrlMode};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    BaseUrl { locations: Vec<Option<String>> },
    VertexUrl { cases: Vec<VertexUrlCase> },
    EmbeddingUrl { cases: Vec<EmbeddingUrlCase> },
    GeminiUrl { cases: Vec<GeminiUrlCase> },
    ModelPredicates { models: Vec<String> },
    UnsupportedModes { modes: Vec<UnsupportedMode> },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnsupportedMode {
    BatchEmbedding,
    ImageGeneration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VertexUrlCase {
    pub(super) mode: ModeName,
    pub(super) model: String,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    pub(super) project: Option<String>,
    #[serde(default)]
    pub(super) location: Option<String>,
    pub(super) api_version: VersionName,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingUrlCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) project: Option<String>,
    #[serde(default)]
    pub(super) location: Option<String>,
    pub(super) api_version: VersionName,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeminiUrlCase {
    pub(super) mode: ModeName,
    pub(super) model: String,
    #[serde(default)]
    pub(super) stream: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModeName {
    Chat,
    Embedding,
    BatchEmbedding,
    ImageGeneration,
    CountTokens,
}

impl ModeName {
    pub(super) const fn into_mode(self) -> UrlMode {
        match self {
            Self::Chat => UrlMode::Chat,
            Self::Embedding => UrlMode::Embedding,
            Self::BatchEmbedding => UrlMode::BatchEmbedding,
            Self::ImageGeneration => UrlMode::ImageGeneration,
            Self::CountTokens => UrlMode::CountTokens,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum VersionName {
    #[serde(rename = "v1")]
    V1,
    #[serde(rename = "v1beta1")]
    V1Beta1,
}

impl VersionName {
    pub(super) const fn into_version(self) -> ApiVersion {
        match self {
            Self::V1 => ApiVersion::V1,
            Self::V1Beta1 => ApiVersion::V1Beta1,
        }
    }
}
