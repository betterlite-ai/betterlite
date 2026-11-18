use betterlite_core::api::responses_stream_primitives::error_fields::StreamErrorObject;
use betterlite_core::api::responses_stream_primitives::failure::{
    map_stream_error_to_exception, mid_stream_fallback_eligible,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::wire_text;

#[derive(Debug, Deserialize)]
pub(crate) struct MappedRow {
    model: String,
    provider: String,
    #[serde(default)]
    fields: IndexMap<String, Json>,
}

pub(crate) fn answer(rows: &[MappedRow]) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        answers.push(row_answer(row)?);
    }
    Ok(json!({ "rows": answers }))
}

fn row_answer(row: &MappedRow) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let class = map_stream_error_to_exception(
        StreamErrorObject::Mapping(&row.fields),
        &row.model,
        &row.provider,
        &redactor,
    );
    let metadata = class.metadata();
    let eligible = mid_stream_fallback_eligible(&class, Some(metadata.status_code));
    Ok(json!({
        "status": metadata.status_code,
        "type": wire_text(&metadata.error_type),
        "param": wire_text(&metadata.param),
        "code": metadata.openai_code,
        "eligible": eligible,
    }))
}
