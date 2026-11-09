pub mod base_url;
pub mod embedding;
pub mod error;
pub mod gemini;
pub mod model_name;
pub mod route;
pub mod vertex;

mod path;
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlMode {
    Chat,
    Embedding,
    BatchEmbedding,
    ImageGeneration,
    CountTokens,
}

impl UrlMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Embedding => "embedding",
            Self::BatchEmbedding => "batch_embedding",
            Self::ImageGeneration => "image_generation",
            Self::CountTokens => "count_tokens",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endpoint {
    GenerateContent,
    StreamGenerateContent,
    EmbedContent,
    Predict,
    CountTokens,
    BatchEmbedContents,
}

impl Endpoint {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GenerateContent => "generateContent",
            Self::StreamGenerateContent => "streamGenerateContent",
            Self::EmbedContent => "embedContent",
            Self::Predict => "predict",
            Self::CountTokens => "countTokens",
            Self::BatchEmbedContents => "batchEmbedContents",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiVersion {
    V1,
    V1Beta1,
}

impl ApiVersion {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V1Beta1 => "v1beta1",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeminiApiVersion {
    V1Alpha,
    V1Beta,
}

impl GeminiApiVersion {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1Alpha => "v1alpha",
            Self::V1Beta => "v1beta",
        }
    }
}

// LiteLLM returns the URL and endpoint name as a tuple; both travel together to the call site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VertexUrl {
    pub url: String,
    pub endpoint: Endpoint,
}
