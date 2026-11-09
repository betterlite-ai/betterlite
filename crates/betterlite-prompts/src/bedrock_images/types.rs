use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceBlock {
    pub bytes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageBlock {
    pub format: String,
    pub source: SourceBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoBlock {
    pub format: String,
    pub source: SourceBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocumentBlock {
    pub format: String,
    pub source: SourceBlock,
    pub name: String,
}

// LiteLLM's ContentBlock TypedDict has optional members; one image conversion fills exactly one
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ContentBlock {
    Image { image: ImageBlock },
    Video { video: VideoBlock },
    Document { document: DocumentBlock },
}
