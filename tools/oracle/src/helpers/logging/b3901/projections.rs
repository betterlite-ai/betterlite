use betterlite_logging::langfuse_observations::attributes::{
    ObservationAttributes, ObservationType, PromptLink, TraceAttributes, observation_attributes,
    trace_attributes,
};
use betterlite_logging::langfuse_observations::identity::Timestamp;
use chrono::DateTime;
use rust_decimal::Decimal;

use super::inputs::{ObservationFields, PromptFields, TraceFields};
use super::values::{invalid, pairs};
use crate::helpers::HelperError;

pub(crate) fn timestamp(
    seconds: Option<Decimal>,
    datetime: Option<&str>,
) -> Result<Option<Timestamp>, Box<HelperError>> {
    if let Some(text) = datetime {
        return Ok(Some(Timestamp::DateTime(parse_datetime(text)?)));
    }
    Ok(seconds.map(Timestamp::Seconds))
}

pub(crate) fn parse_datetime(text: &str) -> Result<DateTime<chrono::Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
        .map_err(|error| invalid(&error.to_string()))
}

pub(crate) fn trace_projection(fields: &TraceFields) -> serde_json::Value {
    pairs(&trace_attributes(&TraceAttributes {
        name: fields.name.as_ref(),
        user_id: fields.user_id.as_ref(),
        session_id: fields.session_id.as_ref(),
        version: fields.version.as_ref(),
        release: fields.release.as_ref(),
        tags: fields.tags.as_ref(),
        metadata: fields.metadata.as_ref(),
        public: fields.public,
        input: fields.input.as_ref(),
        output: fields.output.as_ref(),
    }))
}

pub(crate) fn observation_projection(
    fields: &ObservationFields,
) -> Result<serde_json::Value, Box<HelperError>> {
    let prompt = fields.prompt.as_ref().map(prompt_link);
    Ok(pairs(&observation_attributes(&ObservationAttributes {
        observation_type: observation_type(&fields.observation_type)?,
        input: fields.input.as_ref(),
        output: fields.output.as_ref(),
        metadata: fields.metadata.as_ref(),
        level: fields.level.as_ref(),
        status_message: fields.status_message.as_ref(),
        version: fields.version.as_ref(),
        model: fields.model.as_ref(),
        model_parameters: fields.model_parameters.as_ref(),
        usage_details: fields.usage_details.as_ref(),
        cost_details: fields.cost_details.as_ref(),
        completion_start_time: fields
            .completion_start_time
            .as_deref()
            .map(parse_datetime)
            .transpose()?
            .map(Timestamp::DateTime),
        prompt: prompt.as_ref(),
    })))
}

fn observation_type(text: &str) -> Result<ObservationType, Box<HelperError>> {
    match text {
        "generation" => Ok(ObservationType::Generation),
        "span" => Ok(ObservationType::Span),
        _ => Err(invalid("observation_type must be generation or span")),
    }
}

fn prompt_link(fields: &PromptFields) -> PromptLink {
    PromptLink {
        name: fields.name.clone(),
        version: fields.version,
        is_fallback: fields.is_fallback,
    }
}
