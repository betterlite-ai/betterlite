use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamTransformSink, TranslationCall,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use thiserror::Error;

use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::run::incremental_transform_stream;
use crate::guardrails::unified_stream::{StreamLoopParams, StreamOutput};

pub(crate) struct FakeGuardrail;

impl GuardrailContract for FakeGuardrail {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkUse {
    Absent,
    Empty,
    Carried,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CallRecord {
    pub(crate) sink: SinkUse,
    pub(crate) items: Vec<GuardrailValue>,
    pub(crate) log_target: Option<GuardrailLogTarget>,
}

// What the next call to the translation answers; a missing plan is Pass
pub(crate) enum Plan {
    Pass,
    Rewrite {
        index: i64,
        text: &'static str,
        holdback: i64,
    },
    Mutate,
    Fail(GuardrailFailure),
}

struct FakeState {
    plans: VecDeque<Plan>,
    calls: Vec<CallRecord>,
}

pub(crate) struct FakeTranslation {
    state: Mutex<FakeState>,
    terminator: bool,
}

impl FakeTranslation {
    pub(crate) fn planned(plans: Vec<Plan>) -> Self {
        Self {
            state: Mutex::new(FakeState {
                plans: VecDeque::from(plans),
                calls: Vec::new(),
            }),
            terminator: true,
        }
    }

    pub(crate) fn without_terminator(plans: Vec<Plan>) -> Self {
        Self {
            terminator: false,
            ..Self::planned(plans)
        }
    }

    pub(crate) fn calls(&self) -> Vec<CallRecord> {
        self.state.lock().unwrap().calls.clone()
    }

    fn record_call(
        &self,
        responses_so_far: &[GuardrailValue],
        call: &mut TranslationCall<'_, GuardrailLogTarget>,
    ) -> Plan {
        let mut state = self.state.lock().unwrap();
        let plan = state.plans.pop_front().unwrap_or(Plan::Pass);
        let sink = sink_use(call.stream_transform_sink.as_deref_mut());
        state.calls.push(CallRecord {
            sink,
            items: responses_so_far.to_vec(),
            log_target: call.logging_obj.as_deref().cloned(),
        });
        drop(state);
        plan
    }
}

fn sink_use(sink: Option<&mut StreamTransformSink>) -> SinkUse {
    match sink {
        None => SinkUse::Absent,
        Some(sink)
            if sink.mutated_text_per_choice.is_empty() && sink.holdback_per_choice.is_empty() =>
        {
            SinkUse::Empty
        }
        Some(_) => SinkUse::Carried,
    }
}

impl BaseTranslation for FakeTranslation {
    type Guardrail = FakeGuardrail;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }

    fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        _guardrail: &Self::Guardrail,
        mut call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        let plan = self.record_call(responses_so_far, &mut call);
        let result = match plan {
            Plan::Pass => Ok(()),
            Plan::Rewrite {
                index,
                text,
                holdback,
            } => {
                if let Some(sink) = call.stream_transform_sink.as_deref_mut() {
                    sink.mutated_text_per_choice.insert(index, text.to_owned());
                    sink.holdback_per_choice.insert(index, holdback);
                }
                Ok(())
            }
            Plan::Mutate => {
                mutate_last_content(responses_so_far);
                Ok(())
            }
            Plan::Fail(failure) => Err(failure),
        };
        ready(result)
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<Vec<u8>>> {
        self.terminator
            .then(|| vec![block_summary(failure, stream_started, responses_so_far)])
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        let status = match failure {
            GuardrailFailure::Http { status, .. } | GuardrailFailure::HttpObject { status, .. } => {
                *status
            }
            GuardrailFailure::ModifyResponse(_)
            | GuardrailFailure::GuardrailRaised { .. }
            | GuardrailFailure::BlockedPii { .. }
            | GuardrailFailure::SensitiveDataRoute(_)
            | GuardrailFailure::Other(_) => return None,
        };
        let items = responses_so_far.unwrap_or(&[]);
        Some(vec![GuardrailValue::Text(format!(
            "status={status};items={};texts={}",
            items.len(),
            joined_contents(items)
        ))])
    }
}

fn mutate_last_content(items: &mut [GuardrailValue]) {
    for item in items.iter_mut().rev() {
        if let GuardrailValue::Object(fields) = item
            && let Some(GuardrailValue::Array(choices)) = fields.get_mut("choices")
            && let Some(GuardrailValue::Object(choice)) = choices.first_mut()
            && let Some(GuardrailValue::Object(delta)) = choice.get_mut("delta")
        {
            delta.insert(
                "content".to_owned(),
                GuardrailValue::Text("MUTATED".to_owned()),
            );
            return;
        }
    }
}

fn block_summary(
    failure: &GuardrailFailure,
    stream_started: bool,
    responses_so_far: Option<&[GuardrailValue]>,
) -> Vec<u8> {
    let items = responses_so_far.unwrap_or(&[]);
    let original = match failure {
        GuardrailFailure::ModifyResponse(details) => {
            details.original_response.as_ref().map_or_else(
                || "none".to_owned(),
                |value| serde_json::to_string(value).unwrap_or_default(),
            )
        }
        _ => "none".to_owned(),
    };
    format!(
        "started={stream_started};items={};texts={};original={original}",
        items.len(),
        joined_contents(items)
    )
    .into_bytes()
}

pub(crate) fn joined_contents(items: &[GuardrailValue]) -> String {
    items
        .iter()
        .map(|item| choice_text(item).unwrap_or_default())
        .collect::<Vec<String>>()
        .join("|")
}

fn choice_text(item: &GuardrailValue) -> Option<String> {
    let GuardrailValue::Object(fields) = item else {
        return None;
    };
    let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
        return None;
    };
    let Some(GuardrailValue::Object(choice)) = choices.first() else {
        return None;
    };
    let Some(GuardrailValue::Object(delta)) = choice.get("delta") else {
        return None;
    };
    match delta.get("content") {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

pub(crate) fn object(fields: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    let entries = fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value));
    GuardrailValue::Object(entries.collect())
}

pub(crate) fn chunk_with(choices: Vec<GuardrailValue>) -> GuardrailValue {
    object(vec![
        ("id", GuardrailValue::Text("chatcmpl-1".to_owned())),
        ("created", GuardrailValue::SignedInteger(7)),
        ("model", GuardrailValue::Text("gpt-4o".to_owned())),
        (
            "object",
            GuardrailValue::Text("chat.completion.chunk".to_owned()),
        ),
        ("choices", GuardrailValue::Array(choices)),
    ])
}

pub(crate) fn choice(
    index: i64,
    content: GuardrailValue,
    role: GuardrailValue,
    tool_calls: GuardrailValue,
    finish_reason: Option<&str>,
) -> GuardrailValue {
    object(vec![
        ("index", GuardrailValue::SignedInteger(index)),
        (
            "delta",
            object(vec![
                ("content", content),
                ("role", role),
                ("tool_calls", tool_calls),
            ]),
        ),
        (
            "finish_reason",
            finish_reason.map_or(GuardrailValue::Null, |reason| {
                GuardrailValue::Text(reason.to_owned())
            }),
        ),
    ])
}

pub(crate) fn tool_calls() -> GuardrailValue {
    GuardrailValue::Array(vec![object(vec![
        ("index", GuardrailValue::SignedInteger(0)),
        ("id", GuardrailValue::Text("call_0".to_owned())),
        ("type", GuardrailValue::Text("function".to_owned())),
        (
            "function",
            object(vec![
                ("name", GuardrailValue::Text("lookup".to_owned())),
                ("arguments", GuardrailValue::Text("{}".to_owned())),
            ]),
        ),
    ])])
}

fn delta_chunk(
    content: GuardrailValue,
    tool_calls: GuardrailValue,
    finish_reason: Option<&str>,
) -> GuardrailValue {
    chunk_with(vec![choice(
        0,
        content,
        GuardrailValue::Null,
        tool_calls,
        finish_reason,
    )])
}

pub(crate) fn text_chunk(text: &str) -> GuardrailValue {
    delta_chunk(
        GuardrailValue::Text(text.to_owned()),
        GuardrailValue::Null,
        None,
    )
}

pub(crate) fn text_chunk_finishing(text: &str, reason: &str) -> GuardrailValue {
    let content = GuardrailValue::Text(text.to_owned());
    delta_chunk(content, GuardrailValue::Null, Some(reason))
}

pub(crate) fn tool_chunk() -> GuardrailValue {
    delta_chunk(GuardrailValue::Null, tool_calls(), None)
}

pub(crate) fn mixed_tool_chunk(text: &str, reason: Option<&str>) -> GuardrailValue {
    let content = GuardrailValue::Text(text.to_owned());
    delta_chunk(content, tool_calls(), reason)
}

pub(crate) fn metadata_chunk() -> GuardrailValue {
    let mut fields = BTreeMap::new();
    fields.insert(
        "id".to_owned(),
        GuardrailValue::Text("chatcmpl-meta".to_owned()),
    );
    fields.insert("created".to_owned(), GuardrailValue::SignedInteger(99));
    fields.insert(
        "model".to_owned(),
        GuardrailValue::Text("meta-model".to_owned()),
    );
    fields.insert(
        "object".to_owned(),
        GuardrailValue::Text("chat.completion.chunk".to_owned()),
    );
    fields.insert("choices".to_owned(), GuardrailValue::Array(Vec::new()));
    fields.insert(
        "usage".to_owned(),
        object(vec![("total_tokens", GuardrailValue::SignedInteger(5))]),
    );
    GuardrailValue::Object(fields)
}

// A chunk build_transform_chunk writes: id/created/model come from the reference chunk
pub(crate) fn synthetic(
    text: &str,
    role: Option<&str>,
    finish_reason: Option<&str>,
) -> GuardrailValue {
    chunk_with(vec![choice(
        0,
        GuardrailValue::Text(text.to_owned()),
        role.map_or(GuardrailValue::Null, |given| {
            GuardrailValue::Text(given.to_owned())
        }),
        GuardrailValue::Null,
        finish_reason,
    )])
}

pub(crate) fn terminator(reason: &str) -> GuardrailValue {
    let content = GuardrailValue::Text(String::new());
    delta_chunk(content, GuardrailValue::Null, Some(reason))
}

pub(crate) fn modify_response(original_response: Option<GuardrailValue>) -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: "blocked".to_owned(),
        model: "gpt-4o".to_owned(),
        request_data: BTreeMap::new(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response,
    }))
}

pub(crate) const fn item(value: GuardrailValue) -> StreamOutput {
    StreamOutput::Item(value)
}

pub(crate) fn block_texts<E>(outputs: &[Result<StreamOutput, StreamLoopError<E>>]) -> Vec<String> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Ok(StreamOutput::BlockChunks(chunks)) => {
                Some(String::from_utf8(chunks.concat()).unwrap())
            }
            Ok(StreamOutput::Item(_)) | Err(_) => None,
        })
        .collect()
}

pub(crate) fn params<'run>(
    translation: &'run FakeTranslation,
    guardrail: &'run FakeGuardrail,
    request_data: &'run mut BTreeMap<String, GuardrailValue>,
    user_api_key_dict: &'run BTreeMap<String, GuardrailValue>,
) -> StreamLoopParams<'run, FakeTranslation, FakeGuardrail> {
    StreamLoopParams {
        translation,
        guardrail,
        request_data,
        user_api_key_dict,
        call_type: CallType::Completion,
        sampling_rate: 5,
        end_of_stream_only: false,
    }
}

// The failure a test source raises where the provider would have dropped the stream
#[derive(Debug, PartialEq, Error)]
#[error("the provider dropped the stream")]
pub(crate) struct SourceFailure;

pub(crate) async fn collect_outputs(
    params: StreamLoopParams<'_, FakeTranslation, FakeGuardrail>,
    source: Vec<GuardrailValue>,
) -> Vec<Result<StreamOutput, StreamLoopError<SourceFailure>>> {
    let results = source.into_iter().map(Ok::<_, SourceFailure>);
    collect_results(params, results.collect()).await
}

pub(crate) async fn collect_results(
    params: StreamLoopParams<'_, FakeTranslation, FakeGuardrail>,
    source: Vec<Result<GuardrailValue, SourceFailure>>,
) -> Vec<Result<StreamOutput, StreamLoopError<SourceFailure>>> {
    incremental_transform_stream(params, stream::iter(source))
        .collect()
        .await
}
