use std::collections::BTreeMap;

use betterlite_vertex::endpoint_target::partner::Partner;
use betterlite_vertex::endpoint_target::target::TargetProvider;
use betterlite_vertex::model_urls::{ApiVersion, Endpoint, UrlMode};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    UnsupportedRegionWarning(RegionOperation),
    LlamaOpenapi(PartnerOperation),
    Ai21Stream(PartnerOperation),
    HostAndVersionGrafts(GraftOperation),
    PscRequiredFields(ProxyOperation),
    GeminiCustomKey(GeminiOperation),
    QueryPreserved(ProxyOperation),
    HeaderOverride(HeaderOperation),
    CachePassthroughPath(RouteOperation),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionOperation {
    pub(crate) regions: Vec<RegionCase>,
    #[serde(default)]
    pub(crate) api_bases: Vec<ApiBaseCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionCase {
    pub(crate) model: String,
    pub(crate) region: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApiBaseCase {
    pub(crate) api_base: Option<String>,
    pub(crate) location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartnerOperation {
    #[serde(default)]
    pub(crate) complete: Vec<CompletePartnerCase>,
    #[serde(default)]
    pub(crate) create: Vec<CreatePartnerCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletePartnerCase {
    pub(crate) partner: PartnerName,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) location: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreatePartnerCase {
    pub(crate) partner: PartnerName,
    pub(crate) model: String,
    pub(crate) location: String,
    pub(crate) project: String,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GraftOperation {
    pub(crate) proxies: Vec<ProxyCase>,
    #[serde(default)]
    pub(crate) tokens: Vec<TokenCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyOperation {
    pub(crate) targets: Vec<ProxyCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeminiOperation {
    #[serde(default)]
    pub(crate) tokens: Vec<TokenCase>,
    #[serde(default)]
    pub(crate) proxies: Vec<ProxyCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyCase {
    pub(crate) url: String,
    pub(crate) endpoint: EndpointName,
    #[serde(default)]
    pub(crate) provider: ProviderName,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) auth: Option<String>,
    #[serde(default)]
    pub(crate) gemini_api_key: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) location: Option<String>,
    #[serde(default)]
    pub(crate) version: Option<VersionName>,
    #[serde(default)]
    pub(crate) use_psc_endpoint_format: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenCase {
    #[serde(default)]
    pub(crate) provider: ProviderName,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) auth: Option<String>,
    #[serde(default)]
    pub(crate) gemini_api_key: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) location: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) use_v1beta1_features: bool,
    #[serde(default)]
    pub(crate) mode: ModeName,
    #[serde(default)]
    pub(crate) use_psc_endpoint_format: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderOperation {
    pub(crate) headers: Vec<HeaderCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCase {
    pub(crate) auth: Option<String>,
    #[serde(default)]
    pub(crate) extras: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteOperation {
    #[serde(default)]
    pub(crate) urls: Vec<String>,
    pub(crate) routes: Vec<RouteCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteCase {
    pub(crate) base_url: String,
    pub(crate) route: String,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) location: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderName {
    #[default]
    VertexAi,
    VertexAiBeta,
    Gemini,
}

macro_rules! map_variants {
    ($name:ident, $target:ty, $method:ident {
        $($variant:ident => $value:expr),* $(,)?
    }) => {
        impl $name {
            pub(crate) const fn $method(self) -> $target {
                match self {
                    $(Self::$variant => $value,)*
                }
            }
        }
    };
}

map_variants!(ProviderName, TargetProvider, provider {
    VertexAi => TargetProvider::Vertex,
    VertexAiBeta => TargetProvider::Vertex,
    Gemini => TargetProvider::Gemini,
});

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PartnerName {
    Llama,
    Mistralai,
    Ai21,
    Claude,
}

map_variants!(PartnerName, Partner, partner {
    Llama => Partner::Llama,
    Mistralai => Partner::MistralAi,
    Ai21 => Partner::Ai21,
    Claude => Partner::Claude,
});

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModeName {
    #[default]
    Chat,
    Embedding,
    BatchEmbedding,
    ImageGeneration,
    CountTokens,
}

map_variants!(ModeName, UrlMode, mode {
    Chat => UrlMode::Chat,
    Embedding => UrlMode::Embedding,
    BatchEmbedding => UrlMode::BatchEmbedding,
    ImageGeneration => UrlMode::ImageGeneration,
    CountTokens => UrlMode::CountTokens,
});

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum EndpointName {
    #[serde(rename = "generateContent")]
    GenerateContent,
    #[serde(rename = "streamGenerateContent")]
    StreamGenerateContent,
    #[serde(rename = "embedContent")]
    EmbedContent,
    #[serde(rename = "predict")]
    Predict,
    #[serde(rename = "countTokens")]
    CountTokens,
    #[serde(rename = "batchEmbedContents")]
    BatchEmbedContents,
}

map_variants!(EndpointName, Endpoint, endpoint {
    GenerateContent => Endpoint::GenerateContent,
    StreamGenerateContent => Endpoint::StreamGenerateContent,
    EmbedContent => Endpoint::EmbedContent,
    Predict => Endpoint::Predict,
    CountTokens => Endpoint::CountTokens,
    BatchEmbedContents => Endpoint::BatchEmbedContents,
});

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum VersionName {
    #[serde(rename = "v1")]
    V1,
    #[serde(rename = "v1beta1")]
    V1Beta1,
}

map_variants!(VersionName, ApiVersion, version {
    V1 => ApiVersion::V1,
    V1Beta1 => ApiVersion::V1Beta1,
});
