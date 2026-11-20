use betterlite_core::rules::{RuleEvaluator, RulePhase, evaluate};
use betterlite_core::schema::SchemaLimits;
use betterlite_core::schema::validate::validate_schema;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::{CallError, field_error, plain_error};
use super::fields::{field, object_field};
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;

// litellm.enable_json_schema_validation and the bounds B200 validates under
#[derive(Debug, Clone, Copy)]
pub struct Validation {
    pub enable_json_schema_validation: bool,
    pub limits: SchemaLimits,
}

pub struct PostCallInput<'call> {
    pub call_type: &'call str,
    pub model: Option<&'call str>,
    pub request: &'call RequestMap,
    pub outcome: &'call CallOutcome,
    pub validation: Validation,
}

// post_call_processing: post-call rules first, then the two JSON schema validations
pub fn run<E>(rules: &E, input: &PostCallInput<'_>) -> Result<(), CallError>
where
    E: RuleEvaluator,
{
    if !matches!(input.call_type, "completion" | "acompletion") {
        return Ok(());
    }
    let CallOutcome::Response(Json::Object(fields)) = input.outcome else {
        return Ok(());
    };
    let Some(content) = first_message_content(fields) else {
        return Ok(());
    };
    evaluate(
        RulePhase::PostCall,
        rules,
        Some(content),
        input.model.unwrap_or_default(),
    )
    .map_err(|error| CallError::from(error.classify()))?;
    if json_schema_validation_enabled(input.request, input.validation)? {
        validate_response_format(input.request, content, input.validation.limits)?;
    }
    validate_enforced_schema(input.request, content, input.validation.limits)
}

fn first_message_content(fields: &IndexMap<String, Json>) -> Option<&str> {
    let Some(Json::List(choices)) = fields.get("choices") else {
        return None;
    };
    let Some(Json::Object(first)) = choices.first() else {
        return None;
    };
    let Some(Json::Object(message)) = first.get("message") else {
        return None;
    };
    match message.get("content") {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn json_schema_validation_enabled(
    request: &RequestMap,
    validation: Validation,
) -> Result<bool, CallError> {
    let flag: Option<bool> = field(request, "enable_json_schema_validation")?;
    Ok(flag.unwrap_or(validation.enable_json_schema_validation))
}

fn validate_json(schema: &Json, response: &str, limits: SchemaLimits) -> Result<(), CallError> {
    let encoded = serde_json::to_value(schema)
        .map_err(|error| CallError::from(plain_error(&error.to_string())))?;
    validate_schema(&encoded, response, &limits)
        .map_err(|error| CallError::from(error.classify(&encoded, response)))
}

fn validate_response_format(
    request: &RequestMap,
    response: &str,
    limits: SchemaLimits,
) -> Result<(), CallError> {
    let Some(format) = object_field(request, "response_format")? else {
        return Ok(());
    };
    let Some(Json::Object(schema_spec)) = format.get("json_schema") else {
        return Ok(());
    };
    let Some(schema) = schema_spec.get("schema") else {
        return Err(field_error(
            "response_format.json_schema.schema",
            "a JSON schema",
        ));
    };
    validate_json(schema, response, limits)
}

fn validate_enforced_schema(
    request: &RequestMap,
    response: &str,
    limits: SchemaLimits,
) -> Result<(), CallError> {
    let Some(format) = object_field(request, "response_format")? else {
        return Ok(());
    };
    if !matches!(format.get("type"), Some(Json::Text(kind)) if kind == "json_object") {
        return Ok(());
    }
    let Some(Json::Object(schema)) = format.get("response_schema") else {
        return Ok(());
    };
    if !matches!(format.get("enforce_validation"), Some(Json::Bool(true))) {
        return Ok(());
    }
    validate_json(&Json::Object(schema.clone()), response, limits)
}
