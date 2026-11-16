use std::future::{Future, ready};

use betterlite_core::api::copilot_chat::credentials::CopilotCredentials;
use betterlite_core::api::copilot_responses::model_gate::MergedModelInfo;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::HelperQuery;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use indexmap::IndexMap;
use secrecy::SecretString;

// A cached, unexpired Copilot key record: no login, no refresh, and the endpoint the record names.
pub(super) struct StoredKey {
    pub(super) token: Option<String>,
    pub(super) base: Option<String>,
}

impl CopilotCredentials for StoredKey {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        ready(self.base.clone())
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        ready(Ok(self.token.clone().map(SecretString::from)))
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum LookupError {
    #[error("model {0} is not mapped")]
    NotMapped(String),
    #[error("the model mode could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// Stands in for B2225: the info LiteLLM's lookup resolves is the catalog entry under provider/model.
pub(super) struct CatalogInfo<'catalog> {
    pub(super) catalog: &'catalog ModelCatalog,
}

impl MergedModelInfo for CatalogInfo<'_> {
    type Error = LookupError;

    fn merged_info(
        &mut self,
        query: &HelperQuery<'_>,
    ) -> impl Future<Output = Result<ModelInfo, LookupError>> + Send {
        ready(info_for(self.catalog, query))
    }
}

fn info_for(catalog: &ModelCatalog, query: &HelperQuery<'_>) -> Result<ModelInfo, LookupError> {
    let name = query.custom_llm_provider.map_or_else(
        || query.model.to_owned(),
        |provider| format!("{provider}/{}", query.model),
    );
    let Some(projection) = catalog.metadata(&name) else {
        return Err(LookupError::NotMapped(name));
    };
    let mode = serde_json::to_value(projection.mode)
        .and_then(serde_json::from_value::<Json>)
        .map_err(LookupError::Encode)?;
    let mut fields = IndexMap::new();
    fields.insert(String::from("key"), Json::Text(projection.model.to_owned()));
    fields.insert(String::from("mode"), mode);
    Ok(ModelInfo { fields })
}
