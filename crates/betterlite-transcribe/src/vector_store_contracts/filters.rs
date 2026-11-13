use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreOpenAiParam;
use betterlite_provider::api::vector_store_contracts::config::{Params, VectorStoreConfig};

const SEARCH_PARAMS: [&str; 4] = [
    param_name(VectorStoreOpenAiParam::Filters),
    param_name(VectorStoreOpenAiParam::MaxNumResults),
    param_name(VectorStoreOpenAiParam::RankingOptions),
    param_name(VectorStoreOpenAiParam::RewriteQuery),
];

// VectorStoreCreateOptionalRequestParams' keys, which have no shared enum
const CREATE_PARAMS: [&str; 5] = [
    "name",
    "file_ids",
    "expires_after",
    "chunking_strategy",
    "metadata",
];

const fn param_name(param: VectorStoreOpenAiParam) -> &'static str {
    match param {
        VectorStoreOpenAiParam::Filters => "filters",
        VectorStoreOpenAiParam::MaxNumResults => "max_num_results",
        VectorStoreOpenAiParam::RankingOptions => "ranking_options",
        VectorStoreOpenAiParam::RewriteQuery => "rewrite_query",
    }
}

#[must_use]
pub fn requested_search_params<C>(params: &Params, config: &C) -> Params
where
    C: VectorStoreConfig,
{
    let optional = params
        .iter()
        .filter(|(name, value)| {
            SEARCH_PARAMS.contains(&name.as_str()) && !matches!(value, Json::Null)
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    // LiteLLM gives the mapper the whole request as non_default_params and never drops any of it
    config.map_params(params, optional, false)
}

#[must_use]
pub fn requested_create_params(params: &Params) -> Params {
    params
        .iter()
        .filter(|(name, value)| {
            CREATE_PARAMS.contains(&name.as_str()) && !matches!(value, Json::Null)
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
