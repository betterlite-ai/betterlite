use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream_items::done::{
    create_output_content_part_done_event, create_output_item_done_event,
    create_output_text_done_event, create_reasoning_output_item_done_event,
    create_reasoning_summary_part_done_event, create_reasoning_summary_text_done_event,
};
use betterlite_core::api::chat_response_stream_items::items::{
    create_content_part_added_event, queue_message_item_added_events,
};
use betterlite_core::api::chat_response_stream_items::progression::{
    is_stream_finished, return_default_done_events, return_default_initial_events,
};
use betterlite_core::api::chat_response_stream_items::snapshot::{
    StreamAssembly, create_litellm_model_response, merge_provider_specific_fields, snapshot_chunk,
};
use betterlite_core::api::chat_response_stream_items::sources::EventSources;
use betterlite_core::api::chat_response_stream_tools::fields::{list, object};
use betterlite_core::api::chat_response_stream_tools::state::{
    ChatStreamState, StreamProgress, StreamSetup,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{
    AssemblyError, Request, stream_chunk_builder,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

mod fixture;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fixture::{Hooks, NoCosts, ZeroCounter, failure, hooks};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    request_value: Json,
    responses_api_request: Json,
    custom_llm_provider: Option<String>,
    litellm_metadata: Option<IndexMap<String, Json>>,
    logging_marker: String,
    now: i64,
    uuid_ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Initial,
    MessageItems,
    ContentPart,
    Done {
        target: DoneTarget,
        completion: Json,
    },
    ReasoningDone {
        item_id: String,
        content: String,
        sequence_number: i64,
    },
    DefaultDone {
        completion: Json,
    },
    Pending,
    Snapshot {
        chunk: IndexMap<String, Json>,
        hidden: Option<IndexMap<String, Json>>,
    },
    MergeFields {
        fields: IndexMap<String, Json>,
    },
    Assemble,
    Finished,
    Set {
        #[serde(default)]
        cached_item_id: Option<Json>,
        #[serde(default)]
        cached_response_id: Option<Json>,
        #[serde(default)]
        cached_reasoning_item_id: Option<Json>,
        #[serde(default)]
        sequence_number: Option<Json>,
        #[serde(default)]
        message_output_index: Option<Json>,
        #[serde(default)]
        next_tool_output_index: Option<Json>,
        #[serde(default)]
        litellm_model_response: Option<Json>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoneTarget {
    Text,
    Part,
    Item,
}

// The streaming driver hands the end-of-stream assembly its logging context; the helper supplies the builder itself.
pub(crate) struct AssemblyContext {
    now: i64,
    runtime: Runtime,
    hooks: Hooks,
}

impl StreamAssembly for AssemblyContext {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError> {
        let decoded = chunks
            .iter()
            .map(decode_chunk)
            .collect::<Result<Vec<Chunk>, _>>()?;
        let counter = ZeroCounter;
        let costs = NoCosts;
        let request = Request {
            chunks: Some(&decoded),
            messages: None,
            count_prompt_tokens: None,
            logging: None,
            costs: &costs,
            counter: &counter,
            metadata: None,
            now: self.now,
        };
        self.runtime
            .block_on(stream_chunk_builder(&request, &self.hooks))
    }
}

fn decode_chunk(chunk: &IndexMap<String, Json>) -> Result<Chunk, AssemblyError> {
    let value = serde_json::to_value(chunk)?;
    serde_json::from_value(value).map_err(AssemblyError::from)
}

struct UuidQueue {
    ids: VecDeque<String>,
    missing: i64,
}

impl UuidQueue {
    fn next(&mut self) -> String {
        if let Some(id) = self.ids.pop_front() {
            return id;
        }
        self.missing += 1;
        format!("missing-uuid-{}", self.missing)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let context = AssemblyContext {
            now: input.now,
            runtime,
            hooks: hooks()?,
        };
        let mut state = ChatStreamState::new(StreamSetup {
            model: &input.model,
            logging_context: context,
            request_input: &input.request_value,
            responses_api_request: &input.responses_api_request,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            litellm_metadata: input.litellm_metadata.clone(),
            limits: ApiLimits::default(),
        })
        .or_value_error()?;
        let sources = event_sources(input);
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            let value = apply_step(&mut state, &sources, step)?;
            results.push(json!({
                "step": value,
                "state": state_view(&state, &input.logging_marker),
            }));
        }
        Ok(json!({ "results": results }))
    }
}

fn event_sources(input: &Input) -> EventSources {
    let now = input.now;
    let queue = Arc::new(Mutex::new(UuidQueue {
        ids: input.uuid_ids.iter().cloned().collect(),
        missing: 0,
    }));
    EventSources::new(
        Arc::new(move || now),
        Arc::new(move || {
            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.next()
        }),
    )
}

fn apply_step(
    state: &mut ChatStreamState<AssemblyContext>,
    sources: &EventSources,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Initial => Ok(event_option(
            return_default_initial_events(state).or_value_error()?,
        )),
        Step::MessageItems => {
            queue_message_item_added_events(state, sources).or_value_error()?;
            Ok(pending_events(state))
        }
        Step::ContentPart => Ok(event_view(
            &create_content_part_added_event(state, sources).or_value_error()?,
        )),
        Step::Done { target, completion } => done_step(state, sources, target, completion),
        Step::ReasoningDone {
            item_id,
            content,
            sequence_number,
        } => Ok(reasoning_step(item_id, content, *sequence_number)),
        Step::DefaultDone { completion } => Ok(event_option(
            return_default_done_events(state, sources, completion).or_value_error()?,
        )),
        Step::Pending => Ok(pending_events(state)),
        Step::Snapshot { chunk, hidden } => snapshot_step(state, chunk, hidden.as_ref()),
        Step::MergeFields { fields } => {
            merge_provider_specific_fields(state, fields);
            Ok(json!(state.progress.provider_specific_fields))
        }
        Step::Assemble => {
            let response = create_litellm_model_response(state).or_value_error()?;
            Ok(assembled_view(response.as_ref()))
        }
        Step::Finished => Ok(Value::Bool(is_stream_finished(state))),
        Step::Set { .. } => set_step(state, step),
    }
}

fn done_step(
    state: &mut ChatStreamState<AssemblyContext>,
    sources: &EventSources,
    target: &DoneTarget,
    completion: &Json,
) -> Result<Value, Box<HelperError>> {
    let event = match target {
        DoneTarget::Text => create_output_text_done_event(state, sources, completion),
        DoneTarget::Part => create_output_content_part_done_event(state, sources, completion),
        DoneTarget::Item => create_output_item_done_event(state, sources, completion),
    }
    .or_value_error()?;
    Ok(event_view(&event))
}

fn reasoning_step(item_id: &str, content: &str, sequence_number: i64) -> Value {
    let events = [
        create_reasoning_summary_text_done_event(item_id, content, sequence_number),
        create_reasoning_summary_part_done_event(item_id, content, sequence_number),
        create_reasoning_output_item_done_event(item_id, content, sequence_number),
    ];
    json!(events.iter().map(event_view).collect::<Vec<_>>())
}

fn snapshot_step(
    state: &mut ChatStreamState<AssemblyContext>,
    chunk: &IndexMap<String, Json>,
    hidden: Option<&IndexMap<String, Json>>,
) -> Result<Value, Box<HelperError>> {
    let mut input_chunk = chunk.clone();
    if let Some(hidden_fields) = hidden {
        input_chunk.insert(
            "_hidden_params".to_owned(),
            Json::Object(hidden_fields.clone()),
        );
    }
    let snapshot = snapshot_chunk(&input_chunk);
    let view = snapshot_view(&snapshot);
    let Json::Object(fields) = snapshot else {
        return Err(failure(
            "the chunk snapshot is not a JSON object".to_owned(),
        ));
    };
    state.progress.chunk_snapshots.push(fields);
    Ok(view)
}

fn set_step(
    state: &mut ChatStreamState<AssemblyContext>,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    let Step::Set {
        cached_item_id,
        cached_response_id,
        cached_reasoning_item_id,
        sequence_number,
        message_output_index,
        next_tool_output_index,
        litellm_model_response,
    } = step
    else {
        return Err(failure("the set step lost its fields".to_owned()));
    };
    if let Some(value) = cached_item_id {
        state.progress.cached_item_id = Some(optional_id(value)?);
    }
    if let Some(value) = cached_response_id {
        state.progress.cached_response_id = Some(optional_id(value)?);
    }
    if let Some(value) = cached_reasoning_item_id {
        state.progress.cached_reasoning_item_id = Some(optional_id(value)?);
    }
    if let Some(value) = sequence_number {
        state.progress.sequence_number = optional_count(value)?;
    }
    if let Some(value) = message_output_index {
        state.progress.message_output_index = optional_count(value)?;
    }
    if let Some(value) = next_tool_output_index {
        state.progress.next_tool_output_index = optional_count(value)?;
    }
    if let Some(value) = litellm_model_response {
        state.progress.litellm_model_response = Some(value.clone());
    }
    Ok(Value::Null)
}

fn optional_id(value: &Json) -> Result<String, Box<HelperError>> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(failure("a cached id is not a string".to_owned()))
        }
    }
}

fn optional_count(value: &Json) -> Result<i64, Box<HelperError>> {
    let Json::Number(number) = value else {
        return Err(failure("a counter is not an integer".to_owned()));
    };
    match number.as_i64() {
        Some(count) => Ok(count),
        None => Err(failure("a counter is not an integer".to_owned())),
    }
}

fn event_option(event: Option<KnownResponseEvent>) -> Value {
    match event {
        Some(event) => event_view(&event),
        None => Value::Null,
    }
}

fn event_view(event: &KnownResponseEvent) -> Value {
    serde_json::to_value(event).unwrap_or(Value::Null)
}

fn pending_events(state: &mut ChatStreamState<AssemblyContext>) -> Value {
    let events = std::mem::take(&mut state.progress.pending_response_events);
    json!(events.iter().map(event_view).collect::<Vec<_>>())
}

fn snapshot_view(snapshot: &Json) -> Value {
    let Json::Object(fields) = snapshot else {
        return json!({"usage": null, "_hidden_params": null});
    };
    json!({
        "usage": fields.get("usage"),
        "_hidden_params": fields.get("_hidden_params"),
    })
}

fn assembled_view(response: Option<&Json>) -> Value {
    let Some(Json::Object(assembled)) = response else {
        return Value::Null;
    };
    let content = assembled
        .get("choices")
        .and_then(list)
        .and_then(|choices| choices.first())
        .and_then(object)
        .and_then(|choice| choice.get("message"))
        .and_then(object)
        .and_then(|message| message.get("content"));
    let provider_fields = assembled
        .get("hidden_params")
        .and_then(object)
        .and_then(|hidden| hidden.get("provider_specific_fields"));
    json!({
        "content": content,
        "provider_specific_fields": provider_fields,
    })
}

fn state_view(state: &ChatStreamState<AssemblyContext>, marker: &str) -> Value {
    let progress = &state.progress;
    json!({
        "flags": {
            "sent_response_created": progress.sent_response_created,
            "sent_in_progress": progress.sent_in_progress,
            "sent_output_item_added": progress.sent_output_item_added,
            "sent_message_item_added": progress.sent_message_item_added,
            "sent_content_part_added": progress.sent_content_part_added,
            "sent_output_text_done": progress.sent_output_text_done,
            "sent_content_part_done": progress.sent_content_part_done,
            "sent_output_item_done": progress.sent_output_item_done,
            "sent_annotation_events": progress.sent_annotation_events,
        },
        "counters": {
            "sequence_number": progress.sequence_number,
            "message_output_index": progress.message_output_index,
            "next_tool_output_index": progress.next_tool_output_index,
        },
        "cached_item_id": progress.cached_item_id,
        "cached_response_id": progress.cached_response_id,
        "cached_reasoning_item_id": progress.cached_reasoning_item_id,
        "pending_response_events": pending_view(progress),
        "provider_specific_fields": progress.provider_specific_fields,
        "logging_context": marker,
    })
}

fn pending_view(progress: &StreamProgress) -> Value {
    json!(
        progress
            .pending_response_events
            .iter()
            .map(event_view)
            .collect::<Vec<_>>()
    )
}
