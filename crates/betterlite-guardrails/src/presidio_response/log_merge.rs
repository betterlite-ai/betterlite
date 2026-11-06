use serde_json::value::RawValue;

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::wire::to_raw;

const LOGGING_KEY: &str = "standard_logging_guardrail_information";

pub(super) fn log_entries(request: &RequestData) -> Vec<Box<RawValue>> {
    let bucket = log_bucket_name(request);
    let Some(bucket_raw) = request.get(bucket) else {
        return Vec::new();
    };
    let Ok(Some(metadata)) = serde_json::from_str::<Option<RequestData>>(bucket_raw.get()) else {
        return Vec::new();
    };
    let Some(entries) = metadata.get(LOGGING_KEY) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<Box<RawValue>>>(entries.get())
        .unwrap_or_else(|_error| vec![entries.clone()])
}

pub(super) fn append_log_entries(
    request: &mut RequestData,
    entries: Vec<Box<RawValue>>,
) -> Result<(), PresidioResponseError> {
    if entries.is_empty() {
        return Ok(());
    }
    let bucket = log_bucket_name(request);
    let mut metadata = request
        .get(bucket)
        .map_or_else(RequestData::new, |raw| bucket_metadata(raw));
    let mut logged = metadata.get(LOGGING_KEY).map_or_else(Vec::new, |raw| {
        serde_json::from_str::<Vec<Box<RawValue>>>(raw.get())
            .unwrap_or_else(|_error| vec![raw.clone()])
    });
    logged.extend(entries);
    metadata.insert(LOGGING_KEY.to_owned(), to_raw(&logged)?);
    request.insert(bucket.to_owned(), to_raw(&metadata)?);
    Ok(())
}

pub(super) fn log_bucket_name(request: &RequestData) -> &'static str {
    if request.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

fn bucket_metadata(raw: &RawValue) -> RequestData {
    serde_json::from_str::<Option<RequestData>>(raw.get())
        .ok()
        .flatten()
        .unwrap_or_default()
}
