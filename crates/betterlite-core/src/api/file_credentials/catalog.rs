use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::api::model_credentials::CredentialSource;

// One router model-list row: model_name at the top, and model_info as configured, not ModelInfo's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentRow {
    pub model_name: String,
    pub model_info: JsonObject,
}

impl DeploymentRow {
    // model_info.<field> when it is text; LiteLLM never matches a number, object or list against a name
    #[must_use]
    pub fn info_text(&self, field: &str) -> Option<&str> {
        match self.model_info.get(field)? {
            Json::Text(text) => Some(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

// The Router reads the file credential helpers make besides B10060's lookup, answered by whatever catalog
// the server injects; the credential lookup itself comes from CredentialSource.
pub trait CredentialCatalog: CredentialSource {
    // Router.get_model_names(team_id=...)
    fn model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, Self::Error>;

    // Router.get_model_access_groups()
    fn model_access_groups(&self) -> Result<IndexMap<String, Vec<String>>, Self::Error>;

    // Router.model_list, in deployment order
    fn deployments(&self) -> Result<Vec<DeploymentRow>, Self::Error>;

    // Router.get_credential_deployment(model_id).model_info.model_dump(): the complete resolved
    // model_info an accounting snapshot is priced by
    fn resolved_model_info(&self, model_id: &str) -> Result<Option<JsonObject>, Self::Error>;
}
