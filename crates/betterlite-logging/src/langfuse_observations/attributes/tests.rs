#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use opentelemetry::{Array, KeyValue, StringValue, Value};
use rstest::rstest;
use serde::de::DeserializeOwned;

use super::{
    ObservationAttributes, ObservationType, PromptLink, TraceAttributes, observation_attributes,
    trace_attributes,
};
use crate::langfuse_observations::identity::Timestamp;

#[derive(Clone, Copy, Debug)]
enum Check {
    TraceScalarsKeepOrder,
    TracePublicFlag,
    TraceInputOutputStrings,
    TraceContainerSerializes,
    TraceTagsBecomeArray,
    TraceSingleTagBecomesEntry,
    TraceEmptyTagsAreOmitted,
    TraceMetadataFlattensKeys,
    TraceScalarMetadataRidesUnderPrefix,
    TraceMetadataAbsent,
    TraceNullFieldsAreAbsent,
    ObservationGenerationScalars,
    ObservationSpanType,
    ObservationInputOutputStrings,
    ObservationContainersSerialize,
    ObservationCompletionStartIsJsonText,
    ObservationCompletionStartSecondsStayNumericText,
    ObservationPromptLinks,
    ObservationFallbackPromptIsDropped,
    ObservationMetadataFlattensKeys,
}

fn value(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn fields(text: &str) -> Json {
    value(text)
}

fn text_attr(name: &str, item: &str) -> (String, Value) {
    (name.to_owned(), Value::from(item.to_owned()))
}

fn bool_attr(name: &str, item: bool) -> (String, Value) {
    (name.to_owned(), Value::Bool(item))
}

fn int_attr(name: &str, item: i64) -> (String, Value) {
    (name.to_owned(), Value::I64(item))
}

fn array_attr(name: &str, items: &[&str]) -> (String, Value) {
    (
        name.to_owned(),
        Value::Array(Array::String(
            items
                .iter()
                .map(|item| StringValue::from((*item).to_owned()))
                .collect(),
        )),
    )
}

fn pairs(attributes: Vec<KeyValue>) -> Vec<(String, Value)> {
    attributes
        .into_iter()
        .map(|pair| (pair.key.as_str().to_owned(), pair.value))
        .collect()
}

fn entries(text: &str) -> IndexMap<String, Json> {
    match fields(text) {
        Json::Object(entries) => entries,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("fields are an object")
        }
    }
}

impl<'fields> From<&'fields IndexMap<String, Json>> for TraceAttributes<'fields> {
    fn from(fields: &'fields IndexMap<String, Json>) -> Self {
        Self {
            name: fields.get("name"),
            user_id: fields.get("user_id"),
            session_id: fields.get("session_id"),
            version: fields.get("version"),
            release: fields.get("release"),
            tags: fields.get("tags"),
            metadata: fields.get("metadata"),
            public: public_flag(fields.get("public")),
            input: fields.get("input"),
            output: fields.get("output"),
        }
    }
}

fn trace_from(text: &str) -> Vec<(String, Value)> {
    pairs(trace_attributes(&TraceAttributes::from(&entries(text))))
}

fn public_flag(value: Option<&Json>) -> Option<bool> {
    if let Some(Json::Bool(flag)) = value {
        return Some(*flag);
    }
    None
}

fn decoded<T>(value: Option<&Json>) -> Option<T>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(value?).unwrap();
    Some(serde_json::from_str(&text).unwrap())
}

fn observation_from(text: &str) -> Vec<(String, Value)> {
    let fields = entries(text);
    let prompt = decoded::<PromptLink>(fields.get("prompt"));
    let started = fields.get("completion_start_time").map(|raw| match raw {
        Json::Number(seconds) => Timestamp::Seconds(seconds.to_string().parse().unwrap()),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Timestamp::DateTime(decoded::<DateTime<Utc>>(Some(raw)).unwrap())
        }
    });
    let attributes = ObservationAttributes {
        observation_type: decoded(fields.get("observation_type"))
            .unwrap_or(ObservationType::Generation),
        input: fields.get("input"),
        output: fields.get("output"),
        metadata: fields.get("metadata"),
        level: fields.get("level"),
        status_message: fields.get("status_message"),
        version: fields.get("version"),
        model: fields.get("model"),
        model_parameters: fields.get("model_parameters"),
        usage_details: fields.get("usage_details"),
        cost_details: fields.get("cost_details"),
        completion_start_time: started,
        prompt: prompt.as_ref(),
    };
    pairs(observation_attributes(&attributes))
}

#[rstest]
#[case::trace_scalars_keep_order(Check::TraceScalarsKeepOrder)]
#[case::trace_public_flag(Check::TracePublicFlag)]
#[case::trace_input_output_strings(Check::TraceInputOutputStrings)]
#[case::trace_container_serializes(Check::TraceContainerSerializes)]
#[case::trace_tags_become_array(Check::TraceTagsBecomeArray)]
#[case::trace_single_tag_becomes_entry(Check::TraceSingleTagBecomesEntry)]
#[case::trace_empty_tags_are_omitted(Check::TraceEmptyTagsAreOmitted)]
#[case::trace_metadata_flattens_keys(Check::TraceMetadataFlattensKeys)]
#[case::trace_scalar_metadata_rides_under_prefix(Check::TraceScalarMetadataRidesUnderPrefix)]
#[case::trace_metadata_absent(Check::TraceMetadataAbsent)]
#[case::trace_null_fields_are_absent(Check::TraceNullFieldsAreAbsent)]
#[case::observation_generation_scalars(Check::ObservationGenerationScalars)]
#[case::observation_span_type(Check::ObservationSpanType)]
#[case::observation_input_output_strings(Check::ObservationInputOutputStrings)]
#[case::observation_containers_serialize(Check::ObservationContainersSerialize)]
#[case::observation_completion_start_is_json_text(Check::ObservationCompletionStartIsJsonText)]
#[case::observation_completion_start_seconds_stay_numeric_text(
    Check::ObservationCompletionStartSecondsStayNumericText
)]
#[case::observation_prompt_links(Check::ObservationPromptLinks)]
#[case::observation_fallback_prompt_is_dropped(Check::ObservationFallbackPromptIsDropped)]
#[case::observation_metadata_flattens_keys(Check::ObservationMetadataFlattensKeys)]
fn observation_attributes_tests_behavior(#[case] check: Check) {
    match check {
        Check::TraceScalarsKeepOrder => assert_eq!(
            trace_from(
                r#"{"name":"trace-name","user_id":"user-1","session_id":"session-1","version":"1.0","release":"rel"}"#
            ),
            vec![
                text_attr("langfuse.trace.name", "trace-name"),
                text_attr("user.id", "user-1"),
                text_attr("session.id", "session-1"),
                text_attr("langfuse.version", "1.0"),
                text_attr("langfuse.release", "rel"),
            ]
        ),
        Check::TracePublicFlag => assert_eq!(
            trace_from(r#"{"public":true}"#),
            vec![bool_attr("langfuse.trace.public", true)]
        ),
        Check::TraceInputOutputStrings => assert_eq!(
            trace_from(r#"{"input":"hello","output":"bye"}"#),
            vec![
                text_attr("langfuse.trace.input", "hello"),
                text_attr("langfuse.trace.output", "bye"),
            ]
        ),
        Check::TraceContainerSerializes => assert_eq!(
            trace_from(r#"{"input":{"alpha":1}}"#),
            vec![text_attr("langfuse.trace.input", r#"{"alpha":1}"#)]
        ),
        Check::TraceTagsBecomeArray => assert_eq!(
            trace_from(r#"{"tags":["alpha","beta"]}"#),
            vec![array_attr("langfuse.trace.tags", &["alpha", "beta"])]
        ),
        Check::TraceSingleTagBecomesEntry => assert_eq!(
            trace_from(r#"{"tags":"solo"}"#),
            vec![array_attr("langfuse.trace.tags", &["solo"])]
        ),
        Check::TraceEmptyTagsAreOmitted => assert!(trace_from(r#"{"tags":[]}"#).is_empty()),
        Check::TraceMetadataFlattensKeys => assert_eq!(
            trace_from(
                r#"{"metadata":{"count":1,"flag":true,"ratio":1.5,"big":9007199254740992,"nil":null,"list":["x"]}}"#
            ),
            vec![
                int_attr("langfuse.trace.metadata.count", 1),
                bool_attr("langfuse.trace.metadata.flag", true),
                text_attr("langfuse.trace.metadata.ratio", "1.5"),
                text_attr("langfuse.trace.metadata.big", "9007199254740992"),
                text_attr("langfuse.trace.metadata.list", r#"["x"]"#),
            ]
        ),
        Check::TraceScalarMetadataRidesUnderPrefix => assert_eq!(
            trace_from(r#"{"metadata":"note"}"#),
            vec![text_attr("langfuse.trace.metadata", "note")]
        ),
        Check::TraceMetadataAbsent => assert_eq!(
            trace_from(r#"{"name":"only"}"#),
            vec![text_attr("langfuse.trace.name", "only")]
        ),
        Check::TraceNullFieldsAreAbsent => assert_eq!(
            trace_from(r#"{"name":null,"input":null,"tags":null,"metadata":null,"public":null}"#),
            Vec::new()
        ),
        Check::ObservationGenerationScalars => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","level":"WARNING","status_message":"slow","version":"2","model":"gpt-4o"}"#
            ),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr("langfuse.observation.level", "WARNING"),
                text_attr("langfuse.observation.status_message", "slow"),
                text_attr("langfuse.version", "2"),
                text_attr("langfuse.observation.model.name", "gpt-4o"),
            ]
        ),
        Check::ObservationSpanType => assert_eq!(
            observation_from(r#"{"observation_type":"span"}"#),
            vec![text_attr("langfuse.observation.type", "span")]
        ),
        Check::ObservationInputOutputStrings => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","input":"prompt","output":"answer"}"#
            ),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr("langfuse.observation.input", "prompt"),
                text_attr("langfuse.observation.output", "answer"),
            ]
        ),
        Check::ObservationContainersSerialize => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","model_parameters":{"temperature":0},"usage_details":{"prompt_tokens":3},"cost_details":{}}"#
            ),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr(
                    "langfuse.observation.model.parameters",
                    r#"{"temperature":0}"#
                ),
                text_attr(
                    "langfuse.observation.usage_details",
                    r#"{"prompt_tokens":3}"#
                ),
                text_attr("langfuse.observation.cost_details", "{}"),
            ]
        ),
        Check::ObservationCompletionStartIsJsonText => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","completion_start_time":"2026-01-01T00:00:00Z"}"#
            ),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr(
                    "langfuse.observation.completion_start_time",
                    "\"2026-01-01T00:00:00+00:00\""
                ),
            ]
        ),
        Check::ObservationCompletionStartSecondsStayNumericText => assert_eq!(
            observation_from(r#"{"observation_type":"generation","completion_start_time":1234.5}"#),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr("langfuse.observation.completion_start_time", "1234.5"),
            ]
        ),
        Check::ObservationPromptLinks => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","prompt":{"name":"support-bot","version":3,"is_fallback":false}}"#
            ),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr("langfuse.observation.prompt.name", "support-bot"),
                int_attr("langfuse.observation.prompt.version", 3),
            ]
        ),
        Check::ObservationFallbackPromptIsDropped => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","prompt":{"name":"local","version":1,"is_fallback":true}}"#
            ),
            vec![text_attr("langfuse.observation.type", "generation")]
        ),
        Check::ObservationMetadataFlattensKeys => assert_eq!(
            observation_from(
                r#"{"observation_type":"generation","metadata":{"provider":"openai","cached":false}}"#
            ),
            vec![
                text_attr("langfuse.observation.type", "generation"),
                text_attr("langfuse.observation.metadata.provider", "openai"),
                bool_attr("langfuse.observation.metadata.cached", false),
            ]
        ),
    }
}
