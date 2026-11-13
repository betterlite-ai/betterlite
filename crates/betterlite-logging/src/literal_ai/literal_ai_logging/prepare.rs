use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use indexmap::IndexMap;
use serde_json::Number;

use super::ids::IdSource;
use super::payload::{CallbackKwargs, StandardLoggingPayload};
use super::step::{Generation, Step};

#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("standard_logging_object not found in kwargs")]
    MissingStandardLoggingObject,
    #[error("the step duration does not fit the microsecond range")]
    Duration,
    #[error("the step duration could not be written as a number")]
    Number(#[source] serde_json::Error),
}

#[derive(Default)]
struct PromptAttributes {
    prompt_id: Option<Json>,
    variables: Option<Json>,
}

struct Parts<'event> {
    kwargs: &'event CallbackKwargs,
    payload: &'event StandardLoggingPayload,
    metadata: Option<&'event IndexMap<String, Json>>,
    messages: Option<Json>,
    prompt: PromptAttributes,
    model_parameters: IndexMap<String, Json>,
    tools: Option<Json>,
    micros: i64,
}

// litellm.integrations.literal_ai.LiteralAILogger._prepare_log_data, which ignores the response object it is passed.
pub fn prepare_log_data(
    kwargs: &CallbackKwargs,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    ids: &dyn IdSource,
) -> Result<Step, StepError> {
    let payload = kwargs
        .standard_logging_object
        .as_ref()
        .ok_or(StepError::MissingStandardLoggingObject)?;
    let metadata = kwargs
        .litellm_params
        .as_ref()
        .and_then(|params| params.metadata.as_ref());
    let mut model_parameters = payload.model_parameters.clone();
    let tools = model_parameters.shift_remove("tools");
    let mut messages = payload.messages.clone();
    let prompt = apply_literal_prompts(&mut messages);
    let micros = end_time
        .signed_duration_since(start_time)
        .num_microseconds()
        .ok_or(StepError::Duration)?;
    assemble_step(
        &Parts {
            kwargs,
            payload,
            metadata,
            messages,
            prompt,
            model_parameters,
            tools,
            micros,
        },
        start_time,
        end_time,
        ids,
    )
}

fn assemble_step(
    parts: &Parts<'_>,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    ids: &dyn IdSource,
) -> Result<Step, StepError> {
    let id = parts
        .metadata
        .and_then(|fields| fields.get("step_id"))
        .cloned()
        .unwrap_or_else(|| Json::Text(ids.next_id()));
    let tags = parts
        .metadata
        .and_then(|fields| fields.get("tags").or_else(|| fields.get("literalai_tags")))
        .cloned();
    Ok(Step {
        id,
        error: parts.payload.error_str.clone(),
        name: parts.kwargs.model.clone().unwrap_or_default(),
        thread_id: parts
            .metadata
            .and_then(|fields| fields.get("literalai_thread_id"))
            .cloned(),
        parent_id: parts
            .metadata
            .and_then(|fields| fields.get("literalai_parent_id"))
            .cloned(),
        root_run_id: parts
            .metadata
            .and_then(|fields| fields.get("literalai_root_run_id"))
            .cloned(),
        input: None,
        output: None,
        step_type: "llm".to_owned(),
        tags,
        start_time: start_time.to_rfc3339_opts(SecondsFormat::Micros, true),
        end_time: end_time.to_rfc3339_opts(SecondsFormat::Micros, true),
        metadata: Json::Object(parts.payload.metadata.clone()),
        generation: build_generation(parts)?,
    })
}

fn build_generation(parts: &Parts<'_>) -> Result<Generation, StepError> {
    let provider = parts
        .kwargs
        .custom_llm_provider
        .clone()
        .unwrap_or_else(|| "litellm".to_owned());
    Ok(Generation {
        input_token_count: parts.payload.prompt_tokens,
        output_token_count: parts.payload.completion_tokens,
        token_count: parts.payload.total_tokens,
        prompt_id: parts.prompt.prompt_id.clone(),
        variables: parts.prompt.variables.clone(),
        provider,
        model: parts.kwargs.model.clone().unwrap_or_default(),
        duration: duration_number(parts.micros)?,
        settings: Json::Object(parts.model_parameters.clone()),
        messages: parts.messages.clone(),
        message_completion: choice_message(parts.payload.response.as_ref()),
        tools: parts.tools.clone(),
    })
}

// The last message carrying __literal_prompt__ wins, and it writes uuid and templated onto that message.
fn apply_literal_prompts(messages: &mut Option<Json>) -> PromptAttributes {
    let Some(Json::List(items)) = messages else {
        return PromptAttributes::default();
    };
    if !matches!(items.first(), Some(Json::Object(_))) {
        return PromptAttributes::default();
    }
    let mut prompt = PromptAttributes::default();
    for message in items {
        let Json::Object(fields) = message else {
            continue;
        };
        let Some(Json::Object(literal_prompt)) = fields.get("__literal_prompt__") else {
            continue;
        };
        prompt.prompt_id = literal_prompt.get("prompt_id").cloned();
        prompt.variables = literal_prompt.get("variables").cloned();
        fields.insert(
            "uuid".to_owned(),
            literal_prompt.get("uuid").cloned().unwrap_or(Json::Null),
        );
        fields.insert("templated".to_owned(), Json::Bool(true));
    }
    prompt
}

// response["choices"][0]["message"] when response is an object with a non-empty choices list.
fn choice_message(response: Option<&Json>) -> Option<Json> {
    let Some(Json::Object(fields)) = response else {
        return None;
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        return None;
    };
    let Some(Json::Object(first)) = choices.first() else {
        return None;
    };
    first.get("message").cloned()
}

// (end - start).total_seconds() with the microsecond precision both datetimes carry.
fn duration_number(micros: i64) -> Result<Number, StepError> {
    let sign = if micros < 0 { "-" } else { "" };
    let magnitude = micros.unsigned_abs();
    let seconds = magnitude.div_euclid(1_000_000);
    let mut fraction = format!("{:06}", magnitude.rem_euclid(1_000_000));
    while fraction.ends_with('0') {
        fraction.pop();
    }
    if fraction.is_empty() {
        fraction.push('0');
    }
    Number::from_str(&format!("{sign}{seconds}.{fraction}")).map_err(StepError::Number)
}
