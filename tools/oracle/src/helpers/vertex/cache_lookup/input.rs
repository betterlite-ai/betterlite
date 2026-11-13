use betterlite_vertex::cache_lookup::target::CacheProvider;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) targets: Vec<TargetCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetCase {
    pub(crate) provider: ProviderName,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) location: Option<String>,
    #[serde(default)]
    pub(crate) auth: Option<String>,
    #[serde(default)]
    pub(crate) gemini_api_key: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderName {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

impl ProviderName {
    pub(crate) const fn provider(self) -> CacheProvider {
        match self {
            Self::VertexAi => CacheProvider::Vertex,
            Self::VertexAiBeta => CacheProvider::VertexBeta,
            Self::Gemini => CacheProvider::Gemini,
        }
    }
}
