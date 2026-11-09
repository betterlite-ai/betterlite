use betterlite_openai::stream::chunk::{Choice, Chunk, ChunkError, Delta};
use betterlite_prompts::arguments::json::{Json, Object};

use super::{CHUNK_OBJECT, DEFAULT_CHUNK_ID, DEFAULT_MODEL, SapStreamError};

pub(super) fn final_result_chunk(value: &Json) -> Result<Chunk, SapStreamError> {
    match value {
        Json::Null => chunk_from_object(&Object::new()),
        Json::Object(fields) => chunk_from_object(fields),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(SapStreamError::EventField("final_result"))
        }
    }
}

pub(super) fn chunk_from_object(object: &Object) -> Result<Chunk, SapStreamError> {
    let mut fields = object.clone();
    // LiteLLM's chunk __init__ overwrites object, so a given value and a missing one read the same
    fields.insert(
        String::from("object"),
        Json::Text(String::from(CHUNK_OBJECT)),
    );
    let raw = serde_json::to_value(Json::Object(fields)).map_err(ChunkError::from)?;
    serde_json::from_value(raw)
        .map_err(ChunkError::from)
        .map_err(SapStreamError::from)
}

pub(super) fn orchestration_chunk(
    result: &Json,
    event: &Object,
    now: i64,
) -> Result<Option<Chunk>, SapStreamError> {
    let Json::Object(orchestration) = result else {
        return if empty_value(result) {
            Ok(None)
        } else {
            Err(SapStreamError::EventField("orchestration_result"))
        };
    };
    if orchestration.is_empty() {
        return Ok(None);
    }
    Ok(Some(Chunk {
        id: orchestration_id(orchestration, event)?,
        object: String::from(CHUNK_OBJECT),
        created: orchestration_created(orchestration, event, now)?,
        model: text(orchestration.get("model"), "orchestration_result.model")?
            .unwrap_or(DEFAULT_MODEL)
            .to_owned(),
        choices: orchestration_choices(orchestration)?,
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    }))
}

fn orchestration_id(orchestration: &Object, event: &Object) -> Result<String, SapStreamError> {
    let result_id = text(orchestration.get("id"), "orchestration_result.id")?;
    if let Some(id) = result_id {
        return Ok(id.to_owned());
    }
    Ok(text(event.get("request_id"), "request_id")?
        .unwrap_or(DEFAULT_CHUNK_ID)
        .to_owned())
}

fn orchestration_created(
    orchestration: &Object,
    event: &Object,
    now: i64,
) -> Result<u64, SapStreamError> {
    let result_created = integer(orchestration.get("created"), "orchestration_result.created")?;
    let stamp = match result_created {
        Some(created) => created,
        None => integer(event.get("created"), "created")?.unwrap_or(now),
    };
    u64::try_from(stamp).map_err(|_error| SapStreamError::Timestamp { created: stamp })
}

fn orchestration_choices(orchestration: &Object) -> Result<Vec<Choice>, SapStreamError> {
    match orchestration.get("choices") {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items.iter().map(choice_from).collect::<Result<Vec<_>, _>>(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(SapStreamError::EventField("orchestration_result.choices"))
        }
    }
}

fn choice_from(value: &Json) -> Result<Choice, SapStreamError> {
    let Json::Object(fields) = value else {
        return Err(SapStreamError::EventField("orchestration_result.choices"));
    };
    let index = match fields.get("index") {
        None => 0,
        Some(Json::Number(number)) => {
            let raw = number.as_u64().ok_or(SapStreamError::EventField(
                "orchestration_result.choices.index",
            ))?;
            u32::try_from(raw).map_err(|_error| {
                SapStreamError::EventField("orchestration_result.choices.index")
            })?
        }
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(SapStreamError::EventField(
                "orchestration_result.choices.index",
            ));
        }
    };
    let delta = match fields.get("delta") {
        None | Some(Json::Null) => Delta::default(),
        Some(given @ Json::Object(_)) => delta_object(given)?,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(SapStreamError::EventField(
                "orchestration_result.choices.delta",
            ));
        }
    };
    Ok(Choice {
        index,
        delta,
        finish_reason: text(
            fields.get("finish_reason"),
            "orchestration_result.choices.finish_reason",
        )?
        .map(str::to_owned),
        logprobs: None,
    })
}

fn delta_object(value: &Json) -> Result<Delta, SapStreamError> {
    let raw = serde_json::to_value(value).map_err(ChunkError::from)?;
    serde_json::from_value(raw)
        .map_err(ChunkError::from)
        .map_err(SapStreamError::from)
}

fn text<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value str>, SapStreamError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(SapStreamError::EventField(field))
        }
    }
}

fn integer(value: Option<&Json>, field: &'static str) -> Result<Option<i64>, SapStreamError> {
    let number = match value {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Number(number)) => number,
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(SapStreamError::EventField(field));
        }
    };
    number
        .as_i64()
        .map(Some)
        .ok_or(SapStreamError::EventField(field))
}

fn empty_value(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
        Json::Bool(_) | Json::Number(_) => false,
    }
}
