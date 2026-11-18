use betterlite_db::budget_permission_rows::JsonData;

use super::caller::CallerAccess;
use super::ports::GeneralSettingsReader;
use super::record::EndpointRecordError;
use super::record::PassThroughGenericEndpoint;

pub const PASS_THROUGH_FIELD: &str = "pass_through_endpoints";

// LiteLLM catches every failure of the general settings read, the caller's missing admin view included, and answers with no database endpoints
pub async fn database_endpoints(
    reader: &dyn GeneralSettingsReader,
    endpoint_id: Option<&str>,
    access: Option<CallerAccess>,
) -> Result<Vec<PassThroughGenericEndpoint>, EndpointRecordError> {
    let Some(stored) = read_stored(reader, access).await else {
        return Ok(Vec::new());
    };
    match stored {
        JsonData::Null => Ok(Vec::new()),
        JsonData::Array(entries) => match endpoint_id {
            Some(id) => Ok(find_endpoint_by_id(&entries, id)?.into_iter().collect()),
            None => entries
                .iter()
                .filter(|entry| matches!(entry, JsonData::Object(_)))
                .map(|entry| PassThroughGenericEndpoint::from_json(entry, false))
                .collect(),
        },
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_) => {
            Err(EndpointRecordError::NotAList)
        }
    }
}

async fn read_stored(
    reader: &dyn GeneralSettingsReader,
    access: Option<CallerAccess>,
) -> Option<JsonData> {
    let access = access.unwrap_or(CallerAccess::FullAdmin);
    match reader.general_setting(PASS_THROUGH_FIELD, access).await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::debug!(%error, "the stored pass-through endpoints cannot be read");
            None
        }
    }
}

pub fn find_endpoint_by_id(
    entries: &[JsonData],
    endpoint_id: &str,
) -> Result<Option<PassThroughGenericEndpoint>, EndpointRecordError> {
    for entry in entries {
        if !matches!(entry, JsonData::Object(_)) {
            continue;
        }
        let endpoint = PassThroughGenericEndpoint::from_json(entry, false)?;
        if endpoint.id.as_deref() == Some(endpoint_id) {
            return Ok(Some(endpoint));
        }
    }
    Ok(None)
}
