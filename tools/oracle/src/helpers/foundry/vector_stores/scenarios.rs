use std::collections::BTreeMap;

use betterlite_foundry::vector_stores::environment::Params;
use betterlite_foundry::vector_stores::search::SearchQuery;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreTypeEndpointsInput {}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthCredentialsInput {
    #[serde(default)]
    pub(crate) credentials: Vec<Params>,
    #[serde(default)]
    pub(crate) environments: Vec<HeaderScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderScenario {
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchQueryFieldsInput {
    #[serde(default)]
    pub(crate) searches: Vec<SearchScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchScenario {
    pub(crate) vector_store_id: String,
    pub(crate) query: SearchQuery,
    pub(crate) api_base: String,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) litellm_params: Params,
    #[serde(default)]
    pub(crate) extra_body: Option<Params>,
    #[serde(default)]
    pub(crate) embedding: Option<Vec<f64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorQueryWithEmbeddingInput {
    #[serde(default)]
    pub(crate) scenarios: Vec<EmbeddedSearchScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddedSearchScenario {
    pub(crate) vector_store_id: String,
    pub(crate) query: SearchQuery,
    pub(crate) api_base: String,
    #[serde(default)]
    pub(crate) optional_params: Params,
    #[serde(default)]
    pub(crate) litellm_params: Params,
    pub(crate) embedding: Vec<f64>,
    #[serde(default)]
    pub(crate) asynchronous: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DocumentScoreCitationInput {
    #[serde(default)]
    pub(crate) responses: Vec<CitationScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CitationScenario {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) before_search: Option<BeforeSearch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BeforeSearch {
    pub(crate) vector_store_id: String,
    pub(crate) query: SearchQuery,
    pub(crate) api_base: String,
    #[serde(default)]
    pub(crate) litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnsupportedStoreCreateInput {}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AzureSearchEndpointInput {
    #[serde(default)]
    pub(crate) scenarios: Vec<EndpointScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointScenario {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) litellm_params: Params,
}
