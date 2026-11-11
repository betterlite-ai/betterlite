use betterlite_config::loader::EnvironmentLookup;
use betterlite_context::aws_partition::get_aws_partition;
use indexmap::IndexMap;
use thiserror::Error;

mod tests;

pub const TABLE_SETTING: &str = "dynamodb_table_name";
pub const REGION_VARIABLE: &str = "AWS_REGION_NAME";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("environment variable AWS_REGION_NAME is not set")]
    MissingRegion,
    #[error(
        "LiteLLM Error, trying to use DynamoDB but not table name passed. Create a table and set `litellm.dynamodb_table_name=<your-table>`"
    )]
    MissingTableName,
    #[error("the DynamoDB callback setting '{name}' is not supported")]
    UnknownKey { name: String },
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub table_name: String,
    pub region_name: String,
}

pub fn resolve(
    params: &IndexMap<String, String>,
    environment: &dyn EnvironmentLookup,
) -> Result<Settings, SettingsError> {
    let region_name = environment
        .lookup(REGION_VARIABLE)
        .filter(|value| !value.is_empty())
        .ok_or(SettingsError::MissingRegion)?;
    let mut remaining = params.clone();
    let table_name = remaining
        .shift_remove(TABLE_SETTING)
        .ok_or(SettingsError::MissingTableName)?;
    if let Some(name) = remaining.keys().next() {
        return Err(SettingsError::UnknownKey { name: name.clone() });
    }
    Ok(Settings {
        table_name,
        region_name,
    })
}

// boto3.resource builds the regional endpoint from the same region and partition pair
#[must_use]
pub fn endpoint_url(region_name: &str) -> String {
    format!(
        "https://dynamodb.{region_name}.{}/",
        get_aws_partition(Some(region_name)).dns_suffix
    )
}
