use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiteLlmCacheType {
    Local,
    Redis,
    RedisSemantic,
    ValkeySemantic,
    S3,
    Disk,
    QdrantSemantic,
    AzureBlob,
    Gcs,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    DefaultOn,
    DefaultOff,
}
