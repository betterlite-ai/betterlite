pub mod base_url;
pub mod embedding;
pub mod error;
pub mod gemini;
pub mod model_name;
pub mod route;
pub mod vertex;

mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlMode {
    Chat,
    Embedding,
    BatchEmbedding,
    ImageGeneration,
    CountTokens,
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

// LiteLLM returns the URL and endpoint name as a tuple; both travel together to the call site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VertexUrl {
    pub url: String,
    pub endpoint: &'static str,
}
