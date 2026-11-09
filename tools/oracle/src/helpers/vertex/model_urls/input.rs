use betterlite_vertex::model_urls::error::VertexUrlError;
use betterlite_vertex::model_urls::{ApiVersion, UrlMode};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    BaseUrl { locations: Vec<Option<String>> },
    Urls { cases: Vec<UrlCase> },
    ModelPredicates { models: Vec<String> },
    UnsupportedModes { modes: Vec<UnsupportedMode> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    pub(super) kind: UrlKind,
    pub(super) model: String,
    #[serde(default)]
    pub(super) mode: Option<ModeName>,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    pub(super) project: Option<String>,
    #[serde(default)]
    pub(super) location: Option<String>,
    #[serde(default)]
    pub(super) api_version: Option<VersionName>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UrlKind {
    Vertex,
    Embedding,
    Gemini,
}

impl UrlKind {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Embedding => "embedding",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CaseError {
    #[error("a {kind} case needs a mode")]
    MissingMode { kind: &'static str },
    #[error("a {kind} case needs an api_version")]
    MissingVersion { kind: &'static str },
    #[error("url error: {0}")]
    Url(#[from] VertexUrlError),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnsupportedMode {
    BatchEmbedding,
    ImageGeneration,
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
