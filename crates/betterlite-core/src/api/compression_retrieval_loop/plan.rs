use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::to_raw_value;
use thiserror::Error;

use crate::api::compression_interception::engine::CompressionEngine;
use crate::api::compression_interception::hook::{CompressionInterception, resolve_call_id};
use crate::api::compression_interception::tools::CONTENT_RETRIEVE_TOOL_NAME;

use super::gate::block_input;
use super::{CONTENT_FIELD, ID_FIELD, INPUT_FIELD, NAME_FIELD, TOOL_USE_TYPE, TYPE_FIELD};

const TOOL_CALLS_FIELD: &str = "tool_calls";
const THINKING_BLOCKS_FIELD: &str = "thinking_blocks";
const TOOL_TYPE_FIELD: &str = "tool_type";
const TOOL_RESULT_TYPE: &str = "tool_result";
const TOOL_USE_ID_FIELD: &str = "tool_use_id";
const ROLE_FIELD: &str = "role";
const ASSISTANT_ROLE: &str = "assistant";
const USER_ROLE: &str = "user";
const MAX_TOKENS_FIELD: &str = "max_tokens";
const MODEL_FIELD: &str = "model";
const KEY_FIELD: &str = "key";
const CALL_ID_FIELD: &str = "call_id";
const COMPRESSION_RETRIEVAL_TYPE: &str = "compression_retrieval";
const INTERNAL_KWARGS_PREFIX: &str = "_compression_interception";
const LOGGING_OBJECT_FIELD: &str = "litellm_logging_obj";
const NO_RETRIEVAL_KEY: &str = "No retrieval key provided.";

pub struct RetrievalLoopRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request IndexMap<String, Json>,
    pub logging: Option<&'request AgenticLogging>,
    pub kwargs: &'request IndexMap<String, Json>,
}

#[derive(Debug, Error)]
pub enum RetrievalLoopError {
    #[error("could not serialize a retrieval loop value")]
    Serialize(#[from] serde_json::Error),
    #[error("the {field} field does not hold the type LiteLLM declares for it")]
    FieldType { field: &'static str },
}

impl<Engine> CompressionInterception<Engine>
where
    Engine: CompressionEngine,
{
    pub fn build_agentic_loop_plan(
        &self,
        tools: &IndexMap<String, Json>,
        request: &RetrievalLoopRequest<'_>,
    ) -> Result<AgenticLoopPlan, RetrievalLoopError> {
        self.cache.prune();
        let tool_calls = loop_field(tools, TOOL_CALLS_FIELD)?;
        let thinking_blocks = loop_field(tools, THINKING_BLOCKS_FIELD)?;
        let call_id = resolve_call_id(
            request
                .logging
                .and_then(|logging| logging.call_id.as_deref()),
            request.kwargs,
        );
        let cache = self.get_cache(call_id);
        let mut contents = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            contents.push(retrieval_content(call, &cache)?);
        }
        let patch = AgenticLoopRequestPatch {
            model: Some(patch_model(request.logging, request.model)?),
            messages: Some(follow_up_messages(
                request.messages,
                thinking_blocks,
                tool_calls,
                &contents,
            )?),
            tools: None,
            max_tokens: patch_max_tokens(request.optional_params, request.kwargs)?,
            optional_params: without_max_tokens(request.optional_params)?,
            kwargs: followup_kwargs(request.kwargs)?,
        };
        Ok(AgenticLoopPlan {
            run_agentic_loop: true,
            request_patch: Some(patch),
            response_override: None,
            terminate: false,
            stop_reason: None,
            metadata: plan_metadata(call_id)?,
        })
    }
}

fn loop_field<'tools>(
    tools: &'tools IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'tools [Json], RetrievalLoopError> {
    match tools.get(field) {
        None => Ok(&[]),
        Some(Json::List(items)) => Ok(items.as_slice()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(RetrievalLoopError::FieldType { field })
        }
    }
}

fn call_fields(call: &Json) -> Result<&IndexMap<String, Json>, RetrievalLoopError> {
    match call {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(RetrievalLoopError::FieldType {
                field: TOOL_CALLS_FIELD,
            })
        }
    }
}

pub(super) fn retrieval_content(
    call: &Json,
    cache: &IndexMap<String, String>,
) -> Result<String, RetrievalLoopError> {
    let fields = call_fields(call)?;
    let Some(Json::Object(input)) = fields.get(INPUT_FIELD) else {
        return Ok(NO_RETRIEVAL_KEY.to_owned());
    };
    let Some(Json::Text(key)) = input.get(KEY_FIELD) else {
        return Ok(NO_RETRIEVAL_KEY.to_owned());
    };
    if key.is_empty() {
        return Ok(NO_RETRIEVAL_KEY.to_owned());
    }
    match cache.get(key) {
        Some(content) => Ok(content.clone()),
        None => Ok(format!("[compressed content key '{key}' not found]")),
    }
}

fn follow_up_messages(
    original: &[Json],
    thinking_blocks: &[Json],
    tool_calls: &[Json],
    contents: &[String],
) -> Result<Vec<FollowupMessage>, RetrievalLoopError> {
    let mut messages = Vec::with_capacity(original.len().saturating_add(2));
    for message_value in original {
        messages.push(FollowupMessage(to_raw_value(message_value)?));
    }
    let assistant = assistant_message(thinking_blocks, tool_calls)?;
    messages.push(FollowupMessage(to_raw_value(&assistant)?));
    let user = user_message(tool_calls, contents)?;
    messages.push(FollowupMessage(to_raw_value(&user)?));
    Ok(messages)
}

fn assistant_message(
    thinking_blocks: &[Json],
    tool_calls: &[Json],
) -> Result<Json, RetrievalLoopError> {
    let mut content = thinking_blocks.to_vec();
    for call in tool_calls {
        content.push(tool_use_block(call)?);
    }
    Ok(message(ASSISTANT_ROLE, Json::List(content)))
}

fn tool_use_block(call: &Json) -> Result<Json, RetrievalLoopError> {
    let fields = call_fields(call)?;
    let mut block = IndexMap::with_capacity(4);
    block.insert(TYPE_FIELD.to_owned(), Json::Text(TOOL_USE_TYPE.to_owned()));
    block.insert(
        ID_FIELD.to_owned(),
        fields.get(ID_FIELD).cloned().unwrap_or(Json::Null),
    );
    block.insert(NAME_FIELD.to_owned(), call_name(fields));
    block.insert(INPUT_FIELD.to_owned(), block_input(fields));
    Ok(Json::Object(block))
}

fn call_name(fields: &IndexMap<String, Json>) -> Json {
    match fields.get(NAME_FIELD) {
        None | Some(Json::Null) => Json::Text(CONTENT_RETRIEVE_TOOL_NAME.to_owned()),
        Some(name) => name.clone(),
    }
}

fn user_message(tool_calls: &[Json], contents: &[String]) -> Result<Json, RetrievalLoopError> {
    let mut results = Vec::with_capacity(tool_calls.len());
    for (call, content) in tool_calls.iter().zip(contents) {
        let fields = call_fields(call)?;
        let mut block = IndexMap::with_capacity(3);
        block.insert(
            TYPE_FIELD.to_owned(),
            Json::Text(TOOL_RESULT_TYPE.to_owned()),
        );
        block.insert(
            TOOL_USE_ID_FIELD.to_owned(),
            fields.get(ID_FIELD).cloned().unwrap_or(Json::Null),
        );
        block.insert(CONTENT_FIELD.to_owned(), Json::Text(content.clone()));
        results.push(Json::Object(block));
    }
    Ok(message(USER_ROLE, Json::List(results)))
}

fn message(role: &str, content: Json) -> Json {
    let mut fields = IndexMap::with_capacity(2);
    fields.insert(ROLE_FIELD.to_owned(), Json::Text(role.to_owned()));
    fields.insert(CONTENT_FIELD.to_owned(), content);
    Json::Object(fields)
}

fn patch_max_tokens(
    optional_params: &IndexMap<String, Json>,
    kwargs: &IndexMap<String, Json>,
) -> Result<Option<u64>, RetrievalLoopError> {
    if let Some(count) =
        count_field(optional_params.get(MAX_TOKENS_FIELD))?.filter(|count| *count != 0)
    {
        return Ok(Some(count));
    }
    count_field(kwargs.get(MAX_TOKENS_FIELD))
}

fn count_field(value: Option<&Json>) -> Result<Option<u64>, RetrievalLoopError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => match number.as_u64() {
            Some(count) => Ok(Some(count)),
            None => Err(RetrievalLoopError::FieldType {
                field: MAX_TOKENS_FIELD,
            }),
        },
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(RetrievalLoopError::FieldType {
                field: MAX_TOKENS_FIELD,
            })
        }
    }
}

fn without_max_tokens(params: &IndexMap<String, Json>) -> Result<JsonMap, RetrievalLoopError> {
    let mut kept = JsonMap::with_capacity(params.len());
    for (key, value) in params {
        if key.as_str() != MAX_TOKENS_FIELD {
            kept.insert(key.clone(), to_raw_value(value)?);
        }
    }
    Ok(kept)
}

pub(super) fn followup_kwargs(
    kwargs: &IndexMap<String, Json>,
) -> Result<JsonMap, RetrievalLoopError> {
    let mut kept = JsonMap::with_capacity(kwargs.len());
    for (key, value) in kwargs {
        if !key.starts_with(INTERNAL_KWARGS_PREFIX) && key.as_str() != LOGGING_OBJECT_FIELD {
            kept.insert(key.clone(), to_raw_value(value)?);
        }
    }
    Ok(kept)
}

fn patch_model(
    logging: Option<&AgenticLogging>,
    model: &str,
) -> Result<String, RetrievalLoopError> {
    let Some(params) = logging.and_then(|record| record.agentic_loop_params.as_ref()) else {
        return Ok(model.to_owned());
    };
    let Some(raw) = params.get(MODEL_FIELD) else {
        return Ok(model.to_owned());
    };
    let recorded: Option<String> = serde_json::from_str(raw.get())
        .map_err(|_error| RetrievalLoopError::FieldType { field: MODEL_FIELD })?;
    match recorded {
        Some(recorded_model) => Ok(recorded_model),
        None => Ok(model.to_owned()),
    }
}

fn plan_metadata(call_id: Option<&str>) -> Result<JsonMap, RetrievalLoopError> {
    let mut metadata = JsonMap::with_capacity(2);
    metadata.insert(
        TOOL_TYPE_FIELD.to_owned(),
        to_raw_value(&COMPRESSION_RETRIEVAL_TYPE)?,
    );
    metadata.insert(
        CALL_ID_FIELD.to_owned(),
        to_raw_value(&call_id.unwrap_or_default())?,
    );
    Ok(metadata)
}
