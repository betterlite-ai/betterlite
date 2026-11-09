pub mod base_url;
pub mod embedding;
pub mod error;
pub mod gemini;
pub mod model_name;
pub mod route;
pub mod vertex;

mod path;
mod tests;

// The four closed vocabularies below carry a wire spelling; one implementation keeps them in step.
macro_rules! wire_names {
    ($enum:ident { $($variant:ident => $wire:literal),* $(,)? }) => {
        impl $enum {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)*
                }
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlMode {
    Chat,
    Embedding,
    BatchEmbedding,
    ImageGeneration,
    CountTokens,
}

wire_names!(UrlMode {
    Chat => "chat",
    Embedding => "embedding",
    BatchEmbedding => "batch_embedding",
    ImageGeneration => "image_generation",
    CountTokens => "count_tokens",
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endpoint {
    GenerateContent,
    StreamGenerateContent,
    EmbedContent,
    Predict,
    CountTokens,
    BatchEmbedContents,
}

wire_names!(Endpoint {
    GenerateContent => "generateContent",
    StreamGenerateContent => "streamGenerateContent",
    EmbedContent => "embedContent",
    Predict => "predict",
    CountTokens => "countTokens",
    BatchEmbedContents => "batchEmbedContents",
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiVersion {
    V1,
    V1Beta1,
}

wire_names!(ApiVersion {
    V1 => "v1",
    V1Beta1 => "v1beta1",
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeminiApiVersion {
    V1Alpha,
    V1Beta,
}

wire_names!(GeminiApiVersion {
    V1Alpha => "v1alpha",
    V1Beta => "v1beta",
});

// LiteLLM returns the URL and endpoint name as a tuple; both travel together to the call site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VertexUrl {
    pub url: String,
    pub endpoint: Endpoint,
}
