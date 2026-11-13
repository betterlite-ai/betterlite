use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AffinityMetadata {
    pub model_info: Option<ModelInfo>,
    #[serde(rename = "encrypted_content_affinity_enabled")]
    pub affinity_enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ModelInfo {
    pub id: Option<String>,
}
