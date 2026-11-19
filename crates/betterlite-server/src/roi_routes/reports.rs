use std::future::Future;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::config::ConfigError;

use crate::roi_cache::store::ReportRepository;
use crate::roi_settings::repository::SettingsRepository;

// the sync runner needs its repository by value; the Arc is the only object-safe way to keep it
impl ReportRepository for Arc<dyn SettingsRepository> {
    type Error = ConfigError;

    fn get_param(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, ConfigError>> + Send {
        let repository = Arc::clone(self);
        async move { repository.param(param_name).await }
    }

    fn set_param(
        &self,
        param_name: &str,
        param_value: JsonData,
    ) -> impl Future<Output = Result<(), ConfigError>> + Send {
        let repository = Arc::clone(self);
        async move { SettingsRepository::set_param(repository.as_ref(), param_name, param_value).await }
    }
}
