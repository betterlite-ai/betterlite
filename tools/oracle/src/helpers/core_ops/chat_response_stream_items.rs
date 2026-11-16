use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream_items::done::{
    MessageDone, ReasoningDone, message_done_event, reasoning_done_event,
};
use betterlite_core::api::chat_response_stream_items::error::StreamItemsError;
use betterlite_core::api::chat_response_stream_items::items::{
    content_part_added, queue_message_added,
};
use betterlite_core::api::chat_response_stream_items::progression::{
    is_stream_finished, return_default_done_events, return_default_initial_events,
};
use betterlite_core::api::chat_response_stream_items::snapshot::{
    create_litellm_model_response, merge_provider_specific_fields, snapshot_chunk,
};
use betterlite_core::api::chat_response_stream_items::sources::EventSources;
use betterlite_core::api::chat_response_stream_tools::fields::{list, object};
use betterlite_core::api::chat_response_stream_tools::state::{ChatStreamState, StreamSetup};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

mod fixtures;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fixtures::{AssemblyContext, failure};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    request_value: Json,
    responses_api_request: Json,
    custom_llm_provider: Option<String>,
    litellm_metadata: Option<IndexMap<String, Json>>,
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
    Snapshot {
        chunk: IndexMap<String, Json>,
        hidden: Option<IndexMap<String, Json>>,
    },
    MergeFields {
        fields: IndexMap<String, Json>,
    },
    Assemble,
    Pending,
    Finished,
    Set {
        cached_reasoning_item_id: Option<String>,
        sequence_number: Option<i64>,
        message_output_index: Option<i64>,
        litellm_model_response: Option<Json>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let context = AssemblyContext::new(input.now)?;
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
            results.push(apply_step(&mut state, &sources, step)?);
        }
        Ok(json!({ "results": results }))
    }
}

fn event_sources(input: &Input) -> EventSources {
    let now = input.now;
    let queue = Mutex::new((
        input.uuid_ids.iter().cloned().collect::<VecDeque<_>>(),
        0_i64,
    ));
    EventSources {
        clock: Arc::new(move || Ok::<i64, StreamItemsError>(now)),
        ids: Arc::new(move || {
            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(id) = queue.0.pop_front() {
                return id;
            }
            queue.1 = queue.1.saturating_add(1);
            format!("missing-uuid-{}", queue.1)
        }),
    }
}

fn apply_step(
    state: &mut ChatStreamState<AssemblyContext>,
    sources: &EventSources,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Initial => match return_default_initial_events(state, sources).or_value_error()? {
            Some(event) => serde_json::to_value(&event).or_value_error(),
            None => Ok(Value::Null),
        },
        Step::MessageItems => {
            queue_message_added(state, sources).or_value_error()?;
            pending_events(state)
        }
        Step::ContentPart => {
            let event = content_part_added(state, sources).or_value_error()?;
            serde_json::to_value(&event).or_value_error()
        }
        Step::Done { completion } => done_events(state, sources, completion),
        Step::ReasoningDone {
            item_id,
            content,
            sequence_number,
        } => reasoning_events(item_id, content, *sequence_number),
        Step::DefaultDone { completion } => {
            match return_default_done_events(state, sources, completion).or_value_error()? {
                Some(event) => serde_json::to_value(&event).or_value_error(),
                None => Ok(Value::Null),
            }
        }
        Step::Snapshot { chunk, hidden } => snapshot_step(state, chunk, hidden.as_ref()),
        Step::MergeFields { fields } => {
            merge_provider_specific_fields(state, fields);
            Ok(json!(state.progress.provider_specific_fields))
        }
        Step::Assemble => {
            let response = create_litellm_model_response(state).or_value_error()?;
            Ok(assembled_view(response.as_ref()))
        }
        Step::Pending => pending_events(state),
        Step::Finished => Ok(Value::Bool(is_stream_finished(state))),
        Step::Set { .. } => Ok(set_value(state, step)),
    }
}

fn done_events(
    state: &mut ChatStreamState<AssemblyContext>,
    sources: &EventSources,
    completion: &Json,
) -> Result<Value, Box<HelperError>> {
    let text =
        message_done_event(state, sources, completion, MessageDone::Text).or_value_error()?;
    let part = message_done_event(state, sources, completion, MessageDone::ContentPart)
        .or_value_error()?;
    let item =
        message_done_event(state, sources, completion, MessageDone::Item).or_value_error()?;
    let views = [&text, &part, &item]
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(Value::Array(views))
}

fn reasoning_events(
    item_id: &str,
    content: &str,
    sequence_number: i64,
) -> Result<Value, Box<HelperError>> {
    let events = [
        reasoning_done_event(item_id, content, sequence_number, ReasoningDone::Text),
        reasoning_done_event(item_id, content, sequence_number, ReasoningDone::Part),
        reasoning_done_event(item_id, content, sequence_number, ReasoningDone::Item),
    ];
    let views = events
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(Value::Array(views))
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

fn set_value(state: &mut ChatStreamState<AssemblyContext>, step: &Step) -> Value {
    let Step::Set {
        cached_reasoning_item_id,
        sequence_number,
        message_output_index,
        litellm_model_response,
    } = step
    else {
        return Value::Null;
    };
    if let Some(item_id) = cached_reasoning_item_id {
        state.progress.cached_reasoning_item_id = Some(item_id.clone());
    }
    if let Some(number) = sequence_number {
        state.progress.sequence_number = *number;
    }
    if let Some(index) = message_output_index {
        state.progress.message_output_index = *index;
    }
    if let Some(response) = litellm_model_response {
        state.progress.litellm_model_response = Some(response.clone());
    }
    Value::Null
}

fn pending_events(state: &mut ChatStreamState<AssemblyContext>) -> Result<Value, Box<HelperError>> {
    let events = std::mem::take(&mut state.progress.pending_response_events);
    let views = events
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(Value::Array(views))
}

fn snapshot_view(snapshot: &Json) -> Value {
    let fields = object(snapshot);
    json!({
        "usage": fields.and_then(|fields| fields.get("usage")),
        "_hidden_params": fields.and_then(|fields| fields.get("_hidden_params")),
    })
}

fn assembled_view(response: Option<&Json>) -> Value {
    let Some(Json::Object(assembled)) = response else {
        return Value::Null;
    };
    let message = assembled
        .get("choices")
        .and_then(list)
        .and_then(|choices| choices.first())
        .and_then(object)
        .and_then(|choice| choice.get("message"))
        .and_then(object);
    let provider_fields = assembled
        .get("hidden_params")
        .and_then(object)
        .and_then(|hidden| hidden.get("provider_specific_fields"));
    json!({
        "content": message.and_then(|message| message.get("content")),
        "provider_specific_fields": provider_fields,
    })
}
