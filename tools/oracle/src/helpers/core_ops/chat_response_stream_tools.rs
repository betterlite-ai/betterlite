use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream_tools::final_tools::queue_final_tool_completion;
use betterlite_core::api::chat_response_stream_tools::items::{
    has_signed_thinking_block, output_items_with_id,
};
use betterlite_core::api::chat_response_stream_tools::reasoning::reasoning_ended;
use betterlite_core::api::chat_response_stream_tools::state::{
    ChatStreamState, StreamProgress, StreamSetup, assign_tool_output_index,
};
use betterlite_core::api::chat_response_stream_tools::tool_calls::{
    queue_tool_delta_events, tool_call_id,
};
use betterlite_core::api::chat_response_stream_tools::web_search::reserve_web_search_indexes;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    request_value: Json,
    responses_api_request: Json,
    custom_llm_provider: Option<String>,
    litellm_metadata: Option<IndexMap<String, Json>>,
    logging_marker: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Correlate {
        calls: Vec<Json>,
    },
    Assign {
        call_ids: Vec<String>,
    },
    Deltas {
        tool_calls: Json,
    },
    Final {
        tool_calls: Json,
    },
    Reserve {
        provider_fields: Json,
    },
    ReasoningEnd {
        chunks: Vec<Json>,
    },
    SignedThinking {
        deltas: Vec<Json>,
    },
    ReplaceItems {
        items: Vec<ItemInput>,
        item_type: String,
        item_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ItemInput {
    Typed { item: Json },
    Dict { item: Json },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut state = ChatStreamState::new(StreamSetup {
            model: &input.model,
            logging_context: input.logging_marker.clone(),
            request_input: &input.request_value,
            responses_api_request: &input.responses_api_request,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            litellm_metadata: input.litellm_metadata.clone(),
            limits: ApiLimits::default(),
        })
        .or_value_error()?;
        let mut results = Vec::new();
        for step in &input.steps {
            if let Some(result) = apply_step(&mut state, step)? {
                results.push(result);
            }
        }
        Ok(json!({
            "results": results,
            "state": state_view(&state),
        }))
    }
}

type StepResult = Result<Option<Value>, Box<HelperError>>;

fn apply_step(state: &mut ChatStreamState<String>, step: &Step) -> StepResult {
    match step {
        Step::Correlate { calls } => Ok(Some(correlate_step(state, calls))),
        Step::Assign { call_ids } => assign_step(state, call_ids),
        Step::Deltas { tool_calls } => {
            queue_tool_delta_events(state, tool_calls).or_value_error()?;
            Ok(None)
        }
        Step::Final { tool_calls } => final_step(state, tool_calls),
        Step::Reserve { provider_fields } => {
            reserve_web_search_indexes(state, provider_fields).or_value_error()?;
            Ok(None)
        }
        Step::ReasoningEnd { chunks } => Ok(Some(json!(
            chunks.iter().map(reasoning_ended).collect::<Vec<_>>()
        ))),
        Step::SignedThinking { deltas } => Ok(Some(json!(
            deltas
                .iter()
                .map(has_signed_thinking_block)
                .collect::<Vec<_>>()
        ))),
        Step::ReplaceItems {
            items,
            item_type,
            item_id,
        } => replace_step(items, item_type, item_id.as_deref()),
    }
}

fn correlate_step(state: &mut ChatStreamState<String>, calls: &[Json]) -> Value {
    let ids: Vec<Json> = calls
        .iter()
        .map(|call| Json::Text(tool_call_id(state, call)))
        .collect();
    json!(ids)
}

fn assign_step(state: &mut ChatStreamState<String>, call_ids: &[String]) -> StepResult {
    let mut indices: Vec<i64> = Vec::new();
    for call_id in call_ids {
        indices.push(assign_tool_output_index(state, call_id).or_value_error()?);
    }
    Ok(Some(json!(indices)))
}

fn final_step(state: &mut ChatStreamState<String>, tool_calls: &Json) -> StepResult {
    let payload = json!({"choices": [{"message": {"tool_calls": tool_calls}}]});
    let completion = serde_json::from_value::<Json>(payload).or_value_error()?;
    queue_final_tool_completion(state, &completion).or_value_error()?;
    Ok(None)
}

fn replace_step(items: &[ItemInput], item_type: &str, item_id: Option<&str>) -> StepResult {
    let parsed = items
        .iter()
        .map(parse_item)
        .collect::<Result<Vec<_>, _>>()?;
    let replaced = output_items_with_id(&parsed, item_type, item_id);
    let ids: Vec<Json> = replaced.iter().map(item_id_view).collect();
    Ok(Some(json!(ids)))
}

fn parse_item(input: &ItemInput) -> Result<ResponseOutputItem, Box<HelperError>> {
    let (ItemInput::Typed { item } | ItemInput::Dict { item }) = input;
    let value = serde_json::to_value(item).or_value_error()?;
    if matches!(input, ItemInput::Dict { .. }) {
        let fields = serde_json::from_value::<BTreeMap<String, Json>>(value).or_value_error()?;
        return Ok(ResponseOutputItem::Unknown(fields));
    }
    serde_json::from_value(value).or_value_error()
}

fn item_id_view(item: &ResponseOutputItem) -> Json {
    let Ok(Value::Object(fields)) = serde_json::to_value(item) else {
        return Json::Null;
    };
    fields
        .get("id")
        .and_then(|id| serde_json::from_value::<Json>(id.clone()).ok())
        .unwrap_or(Json::Null)
}

fn state_view(state: &ChatStreamState<String>) -> Value {
    json!({
        "model": state.model,
        "request_input": state.request_input,
        "responses_api_request": state.responses_api_request,
        "custom_llm_provider": state.custom_llm_provider,
        "litellm_metadata": state.litellm_metadata,
        "logging_context": state.logging_context,
        "custom_tool_names": state.custom_tool_names.iter().collect::<Vec<_>>(),
        "namespace_tool_names": namespace_view(state),
        "flags": flags_view(&state.progress),
        "counters": counters_view(&state.progress),
        "queues": queues_view(&state.progress),
    })
}

fn namespace_view(state: &ChatStreamState<String>) -> IndexMap<String, Value> {
    state
        .namespace_tool_names
        .iter()
        .map(|(name, mapped)| (name.clone(), json!([&mapped.namespace, &mapped.name])))
        .collect()
}

fn flags_view(progress: &StreamProgress) -> Value {
    json!({
        "finished": progress.finished,
        "sent_response_created": progress.sent_response_created,
        "sent_output_item_added": progress.sent_output_item_added,
        "sent_content_part_added": progress.sent_content_part_added,
        "upstream_exhausted": progress.upstream_exhausted,
        "final_tool_events_queued": progress.final_tool_events_queued,
    })
}

fn counters_view(progress: &StreamProgress) -> Value {
    json!({
        "sequence_number": progress.sequence_number,
        "message_output_index": progress.message_output_index,
        "next_tool_output_index": progress.next_tool_output_index,
    })
}

fn queues_view(progress: &StreamProgress) -> Value {
    json!({
        "pending_tool_events": progress
            .pending_tool_events
            .iter()
            .map(|queued| serde_json::to_value(&queued.event).unwrap_or(Value::Null))
            .collect::<Vec<_>>(),
        "tool_output_index_by_call_id": progress.tool_output_index_by_call_id,
        "tool_args_by_call_id": progress.tool_args_by_call_id,
        "tool_item_id_by_call_id": progress.tool_item_id_by_call_id,
        "tool_call_id_by_index": progress.tool_call_id_by_index,
        "ambiguous_tool_call_indexes": progress
            .ambiguous_tool_call_indexes
            .iter()
            .collect::<Vec<_>>(),
        "web_search_calls": progress.web_search_calls,
        "queued_web_search_call_ids": progress
            .queued_web_search_call_ids
            .iter()
            .collect::<Vec<_>>(),
    })
}
