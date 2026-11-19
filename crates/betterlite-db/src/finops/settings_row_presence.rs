use crate::auxiliary_repository::repository::PrismaClient;
use crate::budget_permission_rows::JsonData;
use crate::resource_repository::config::ConfigError;
use crate::resource_repository::config::ConfigRepository;
use crate::resource_repository::config::ConfigStoredRow;
use crate::resource_repository::table::KeyFilter;

use super::settings_row::SettingsRowError;

// LiteLLM checks the raw param_value for None without decoding it, so a malformed value still counts
pub async fn settings_row_present(
    client: Option<PrismaClient<'_>>,
    param: &'static str,
) -> Result<bool, SettingsRowError> {
    let rows = ConfigRepository::new(client, false)
        .table()
        .map_err(|source| SettingsRowError::Read {
            param,
            source: ConfigError::Database(source),
        })?
        .as_ref()
        .find::<ConfigStoredRow>(KeyFilter::Equal(param))
        .await
        .map_err(|source| SettingsRowError::Read {
            param,
            source: ConfigError::Table(source),
        })?;
    Ok(rows.iter().any(|row| row.param_value != JsonData::Null))
}
