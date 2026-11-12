#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BedrockEmbeddingFamily {
    TitanTextV1,
    TitanImageV1,
    TitanTextV2,
    CohereEmbed,
    TwelveLabsMarengo,
    Nova,
    Unmapped,
}

#[must_use]
pub fn bedrock_embedding_family(model: &str) -> BedrockEmbeddingFamily {
    if model.contains("amazon.titan-embed-text-v1") {
        BedrockEmbeddingFamily::TitanTextV1
    } else if model.contains("amazon.titan-embed-image-v1") {
        BedrockEmbeddingFamily::TitanImageV1
    } else if model.contains("amazon.titan-embed-text-v2:0") {
        BedrockEmbeddingFamily::TitanTextV2
    } else if model.contains("cohere.embed") {
        BedrockEmbeddingFamily::CohereEmbed
    } else if model.contains("twelvelabs") || model.contains("marengo") {
        BedrockEmbeddingFamily::TwelveLabsMarengo
    } else if model.to_lowercase().contains("nova") {
        BedrockEmbeddingFamily::Nova
    } else {
        BedrockEmbeddingFamily::Unmapped
    }
}
