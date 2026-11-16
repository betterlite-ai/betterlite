use std::fmt::Display;
use std::future::Future;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::HelperQuery;
use betterlite_models::full_model_metadata::record::ModelInfo;

const PROVIDER: &str = "github_copilot";
const RESPONSES_ENDPOINT: &str = "/v1/responses";

// B2225's merged model metadata, so a deployment's own model_info counts
pub trait MergedModelInfo {
    type Error: Display;

    fn merged_info(
        &mut self,
        query: &HelperQuery<'_>,
    ) -> impl Future<Output = Result<ModelInfo, Self::Error>> + Send;
}

// mode responses accepts and mode chat rejects even a dual-endpoint model; any other mode reads the catalog entry at the resolved key
pub async fn supports_responses_api<Source>(
    model: &str,
    source: &mut Source,
    catalog: &ModelCatalog,
) -> bool
where
    Source: MergedModelInfo + Send,
{
    let query = HelperQuery {
        model,
        custom_llm_provider: Some(PROVIDER),
        api_base: None,
    };
    let info = match source.merged_info(&query).await {
        Ok(info) => info,
        Err(error) => {
            tracing::debug!(
                model,
                error = %error,
                "github_copilot_supports_responses_api: get_model_info failed"
            );
            return false;
        }
    };
    let mode = info.fields.get("mode");
    if matches!(mode, Some(Json::Text(text)) if text == "responses") {
        return true;
    }
    if matches!(mode, Some(Json::Text(text)) if text == "chat") {
        return false;
    }
    let Some(Json::Text(key)) = info.fields.get("key") else {
        return false;
    };
    lists_responses_endpoint(catalog, key)
}

// LiteLLM reads the raw model_cost entry at the resolved key; a key without an entry lists nothing
fn lists_responses_endpoint(catalog: &ModelCatalog, key: &str) -> bool {
    let Some(canonical) = catalog.lookup.get(key) else {
        return false;
    };
    catalog
        .entries
        .get(canonical.as_ref())
        .is_some_and(|entry| {
            entry
                .fields
                .metadata
                .supported_endpoints
                .iter()
                .any(|endpoint| endpoint == RESPONSES_ENDPOINT)
        })
}
