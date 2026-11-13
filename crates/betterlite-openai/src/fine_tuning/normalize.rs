use serde_json::{Map, Value};

use super::error::FineTuneError;
use super::job::FineTuneJob;

const AZURE_STATUS: [(&str, &str); 7] = [
    ("pending", "queued"),
    ("notRunning", "queued"),
    ("running", "running"),
    ("succeeded", "succeeded"),
    ("failed", "failed"),
    ("canceled", "cancelled"),
    ("canceling", "cancelled"),
];

pub fn normalize_fine_tuning_job_dict(
    mut data: Map<String, Value>,
    is_azure: bool,
) -> Map<String, Value> {
    if !is_azure {
        return data;
    }
    if data.get("organization_id").is_none_or(Value::is_null) {
        data.insert("organization_id".to_owned(), Value::String(String::new()));
    }
    if data.get("result_files").is_none_or(Value::is_null) {
        data.insert("result_files".to_owned(), Value::Array(Vec::new()));
    }
    let mapped = data
        .get("status")
        .and_then(Value::as_str)
        .and_then(|status| AZURE_STATUS.iter().find(|(azure, _)| *azure == status));
    if let Some((_, openai)) = mapped {
        data.insert("status".to_owned(), Value::String((*openai).to_owned()));
    }
    data
}

pub fn job_from_response(
    response: Map<String, Value>,
    is_azure: bool,
) -> Result<FineTuneJob, FineTuneError> {
    let mut record = normalize_fine_tuning_job_dict(response, is_azure);
    if let Some(Value::Object(error)) = record.get("error")
        && error.values().all(Value::is_null)
    {
        record.insert("error".to_owned(), Value::Null);
    }
    serde_json::from_value(Value::Object(record)).map_err(|source| FineTuneError::Decode { source })
}
