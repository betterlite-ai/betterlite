use std::error::Error;
use std::future::Future;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::model_secrets::PreparedModel;
use betterlite_db::resource_repository::config::ConfigParam;
use betterlite_db::resource_rows::credential::CredentialItem as CredentialRow;
use betterlite_db::resource_rows::model::ProxyModelTable;

// The three tables _rotate_master_key scans and the writes it makes from them, each call failing on its own terms.
pub trait RotationStore: Sync {
    type Error: Error + Send + 'static;

    fn model_rows(&self) -> impl Future<Output = Result<Vec<ProxyModelTable>, Self::Error>> + Send;

    // one transaction that rewrites litellm_params for these model ids and nothing else
    fn save_model_params(
        &self,
        models: &[PreparedModel],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn config_rows(&self) -> impl Future<Output = Result<Vec<ConfigParam>, Self::Error>> + Send;

    fn save_environment_variables(
        &self,
        values: JsonData,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    // the stored rows undecoded, so one row that no longer parses skips only itself
    fn credential_rows(&self) -> impl Future<Output = Result<Vec<JsonData>, Self::Error>> + Send;

    // updated_by is the actor exactly as given, null included
    fn update_credential(
        &self,
        name: &str,
        row: &CredentialRow,
        actor: Option<&str>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
