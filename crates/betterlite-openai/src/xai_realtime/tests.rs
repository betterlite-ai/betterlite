#![cfg(test)]

use betterlite_config::tls::Verification;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::SecretString;

use crate::openai_base_config::Headers;
use crate::realtime::connect::RealtimeConnectSettings;
use crate::realtime::error::RealtimeError;
use crate::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use crate::realtime::ssl::RealtimeSsl;
use crate::realtime::url::QueryParams;

use super::connect::{XAI_API_BASE, XaiRealtimeConnection, prepare_connection};
use super::content_part::MAX_CACHED_CONTENT_PARTS;
use super::error::XaiRealtimeError;
use super::normalizer::{XaiRealtimeNormalizer, should_drop};
use super::session::patch_outgoing_session;

const KEY: &str = "sk-xai-test";
const MODEL: &str = "grok-4-1-fast-non-reasoning";
const MAX_SIZE: usize = 4096;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn normalized(normalizer: &mut XaiRealtimeNormalizer, text: &str) -> Json {
    normalizer
        .normalize(json(text))
        .expect("the event normalizes")
}

fn part_of(event: &Json) -> Json {
    let Json::Object(fields) = event else {
        panic!("the event is an object");
    };
    fields
        .get("part")
        .cloned()
        .expect("the event carries a part")
}

#[derive(Default)]
struct RecordingLogger {
    calls: Vec<String>,
}

impl RealtimeLogger for RecordingLogger {
    fn pre_call(&mut self, record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        self.calls.push(record.api_base.to_owned());
        Ok(())
    }
}

fn prepared(
    api_base: Option<&str>,
    query_params: Option<&QueryParams>,
) -> (XaiRealtimeConnection, RecordingLogger) {
    let key = SecretString::from(KEY);
    let verification = Verification::Enabled;
    let mut logger = RecordingLogger::default();
    let settings = RealtimeConnectSettings {
        api_base,
        api_key: Some(&key),
        model: MODEL,
        query_params,
        beta_realtime: true,
        ssl_verification: &verification,
        max_message_size_bytes: MAX_SIZE,
    };
    let connection = prepare_connection(&settings, &mut logger).expect("the connection prepares");
    (connection, logger)
}

fn connection() -> XaiRealtimeConnection {
    prepared(None, None).0
}

#[test]
fn drops_only_ping_events() {
    assert!(should_drop(&json(r#"{"type":"ping"}"#)));
    assert!(should_drop(&json(r#"{"type":"ping","event_id":"e"}"#)));
    assert!(!should_drop(&json(r#"{"type":"PING"}"#)));
    assert!(!should_drop(&json(r#"{"type":"response.created"}"#)));
    assert!(!should_drop(&json(r#"{"type":5}"#)));
    assert!(!should_drop(&json(r#"[{"type":"ping"}]"#)));
    assert!(!should_drop(&json("null")));
}

#[test]
fn backfills_content_part_done_from_the_cache() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"he"}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_text.done","response_id":"r1","item_id":"i1","text":"hello"}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"hello"}"#));
}

#[test]
fn content_part_added_backfills_a_missing_part() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"audio","transcript":""}}"#,
    );
    let added = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&added), json(r#"{"type":"audio","transcript":""}"#));
}

#[test]
fn missing_parts_default_to_audio_with_an_empty_transcript() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"audio","transcript":""}"#));
    let scalar = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r2","item_id":"i2","part":"missing"}"#,
    );
    assert_eq!(
        part_of(&scalar),
        json(r#"{"type":"audio","transcript":""}"#)
    );
}

#[test]
fn completed_updates_keep_a_previously_declared_type() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"audio","transcript":"he"}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_audio_transcript.done","response_id":"r1","item_id":"i1","transcript":"hello"}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(
        part_of(&done),
        json(r#"{"type":"audio","transcript":"hello"}"#)
    );
}

#[test]
fn completed_updates_keep_other_cached_fields() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"audio","transcript":"he","extra":1}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_audio_transcript.done","response_id":"r1","item_id":"i1","transcript":"hello"}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(
        part_of(&done),
        json(r#"{"type":"audio","transcript":"hello","extra":1}"#)
    );
}

#[test]
fn completed_updates_keep_an_explicit_null_type() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":null}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_text.done","response_id":"r1","item_id":"i1","text":"x"}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":null,"text":"x"}"#));
}

#[test]
fn completed_updates_without_a_cached_part_create_one() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_text.done","response_id":"r1","item_id":"i1","text":"x"}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"x"}"#));
}

#[test]
fn an_absent_completed_value_leaves_the_cached_part_alone() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"he"}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_audio_transcript.done","response_id":"r1","item_id":"i1"}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_text.done","response_id":"r1","item_id":"i1","text":null}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"he"}"#));
}

#[test]
fn reusing_a_key_replaces_its_cached_part() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"first"}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"second"}}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"second"}"#));
}

#[test]
fn an_absent_content_index_keys_as_zero() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"zero"}}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1","content_index":0}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"zero"}"#));
}

#[test]
fn a_null_content_index_keys_apart_from_zero() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"zero"}}"#,
    );
    let null = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1","content_index":null}"#,
    );
    assert_eq!(part_of(&null), json(r#"{"type":"audio","transcript":""}"#));
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i2","content_index":null,"part":{"type":"text","text":"null-key"}}"#,
    );
    let null_key = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i2","content_index":null}"#,
    );
    assert_eq!(
        part_of(&null_key),
        json(r#"{"type":"text","text":"null-key"}"#)
    );
}

#[test]
fn null_ids_key_like_absent_ones() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":null,"item_id":null,"part":{"type":"text","text":"null-ids"}}"#,
    );
    let done = normalized(&mut normalizer, r#"{"type":"response.content_part.done"}"#);
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"null-ids"}"#));
}

#[test]
fn non_text_ids_key_parts_by_value() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":7,"item_id":true,"part":{"type":"text","text":"seven"}}"#,
    );
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":[1,2],"item_id":"i","part":{"type":"text","text":"list"}}"#,
    );
    let seven = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":7,"item_id":true}"#,
    );
    assert_eq!(part_of(&seven), json(r#"{"type":"text","text":"seven"}"#));
    let list = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":[1,2],"item_id":"i"}"#,
    );
    assert_eq!(part_of(&list), json(r#"{"type":"text","text":"list"}"#));
}

#[test]
fn object_ids_match_by_their_entries_not_their_order() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.added","response_id":{"a":1,"b":2},"item_id":"i","part":{"type":"text","text":"ordered"}}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":{"b":2,"a":1},"item_id":"i"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"ordered"}"#));
}

#[test]
fn cached_parts_do_not_cross_connections() {
    let mut first = connection().normalizer;
    let mut second = connection().normalizer;
    normalized(
        &mut first,
        r#"{"type":"response.content_part.added","response_id":"r1","item_id":"i1","part":{"type":"text","text":"one"}}"#,
    );
    let done = normalized(
        &mut second,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i1"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"audio","transcript":""}"#));
}

fn cache_part(
    normalizer: &mut XaiRealtimeNormalizer,
    item: usize,
) -> Result<Json, XaiRealtimeError> {
    let event = format!(
        r#"{{"type":"response.content_part.added","response_id":"r1","item_id":"i{item}","part":{{"type":"text"}}}}"#
    );
    normalizer.normalize(json(&event))
}

fn fill_cache(normalizer: &mut XaiRealtimeNormalizer) {
    let filled: Result<Vec<Json>, XaiRealtimeError> = (0..MAX_CACHED_CONTENT_PARTS)
        .map(|item| cache_part(normalizer, item))
        .collect();
    filled.expect("the cache fills to its bound");
}

#[test]
fn exceeding_the_bound_fails_on_part_cache_limit() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    fill_cache(&mut normalizer);
    let error = cache_part(&mut normalizer, MAX_CACHED_CONTENT_PARTS)
        .expect_err("the first excess key exceeds the bound");
    assert_eq!(
        error.to_string(),
        "the Grok Voice session cached more than 4096 content parts"
    );
    assert!(matches!(
        error,
        XaiRealtimeError::ContentPartLimit { limit } if limit == MAX_CACHED_CONTENT_PARTS
    ));
    normalized(
        &mut normalizer,
        r#"{"type":"response.output_text.done","response_id":"r1","item_id":"i0","text":"kept"}"#,
    );
    let done = normalized(
        &mut normalizer,
        r#"{"type":"response.content_part.done","response_id":"r1","item_id":"i0"}"#,
    );
    assert_eq!(part_of(&done), json(r#"{"type":"text","text":"kept"}"#));
}

#[test]
fn tool_roles_become_assistant_on_any_item_type() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let message = normalized(
        &mut normalizer,
        r#"{"type":"conversation.item.added","item":{"type":"message","role":"tool"}}"#,
    );
    assert_eq!(
        message,
        json(r#"{"type":"conversation.item.added","item":{"type":"message","role":"assistant"}}"#)
    );
    let function_call = normalized(
        &mut normalizer,
        r#"{"type":"conversation.item.added","item":{"type":"function_call","role":"tool","call_id":"c1"}}"#,
    );
    assert_eq!(
        function_call,
        json(
            r#"{"type":"conversation.item.added","item":{"type":"function_call","role":"assistant","call_id":"c1"}}"#
        )
    );
}

#[test]
fn only_conversation_item_added_repairs_roles() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let user = normalized(
        &mut normalizer,
        r#"{"type":"conversation.item.added","item":{"role":"user"}}"#,
    );
    assert_eq!(
        user,
        json(r#"{"type":"conversation.item.added","item":{"role":"user"}}"#)
    );
    let missing_item = normalized(&mut normalizer, r#"{"type":"conversation.item.added"}"#);
    assert_eq!(missing_item, json(r#"{"type":"conversation.item.added"}"#));
    let other_type = normalized(
        &mut normalizer,
        r#"{"type":"conversation.item.done","item":{"role":"tool"}}"#,
    );
    assert_eq!(
        other_type,
        json(r#"{"type":"conversation.item.done","item":{"role":"tool"}}"#)
    );
    let scalar_item = normalized(
        &mut normalizer,
        r#"{"type":"conversation.item.added","item":"tool"}"#,
    );
    assert_eq!(
        scalar_item,
        json(r#"{"type":"conversation.item.added","item":"tool"}"#)
    );
}

#[rstest]
#[case::output_text_delta(
    r#"{"type":"response.output_text.delta","delta":"hi"}"#,
    r#"{"type":"response.output_text.delta","delta":"hi","output_index":0,"content_index":0}"#
)]
#[case::audio_delta_gets_both(
    r#"{"type":"response.output_audio.delta","delta":"aGk="}"#,
    r#"{"type":"response.output_audio.delta","delta":"aGk=","output_index":0,"content_index":0}"#
)]
#[case::output_item_gets_output_only(
    r#"{"type":"response.output_item.added","item":{"type":"message"}}"#,
    r#"{"type":"response.output_item.added","item":{"type":"message"},"output_index":0}"#
)]
#[case::function_arguments_get_output_only(
    r#"{"type":"response.function_call_arguments.delta","delta":"{}"}"#,
    r#"{"type":"response.function_call_arguments.delta","delta":"{}","output_index":0}"#
)]
#[case::output_item_done_gets_output_only(
    r#"{"type":"response.output_item.done","item":{"type":"message"}}"#,
    r#"{"type":"response.output_item.done","item":{"type":"message"},"output_index":0}"#
)]
#[case::function_arguments_done_gets_output_only(
    r#"{"type":"response.function_call_arguments.done","arguments":"{}"}"#,
    r#"{"type":"response.function_call_arguments.done","arguments":"{}","output_index":0}"#
)]
#[case::content_part_added_gets_both(
    r#"{"type":"response.content_part.added","part":{"type":"text","text":"x"}}"#,
    r#"{"type":"response.content_part.added","part":{"type":"text","text":"x"},"output_index":0,"content_index":0}"#
)]
#[case::content_part_done_gets_both(
    r#"{"type":"response.content_part.done","part":{"type":"audio","transcript":"t"}}"#,
    r#"{"type":"response.content_part.done","part":{"type":"audio","transcript":"t"},"output_index":0,"content_index":0}"#
)]
#[case::output_text_done_gets_both(
    r#"{"type":"response.output_text.done","text":"hi"}"#,
    r#"{"type":"response.output_text.done","text":"hi","output_index":0,"content_index":0}"#
)]
#[case::transcript_delta_gets_both(
    r#"{"type":"response.output_audio_transcript.delta","delta":"t"}"#,
    r#"{"type":"response.output_audio_transcript.delta","delta":"t","output_index":0,"content_index":0}"#
)]
#[case::transcript_done_gets_both(
    r#"{"type":"response.output_audio_transcript.done","transcript":"t"}"#,
    r#"{"type":"response.output_audio_transcript.done","transcript":"t","output_index":0,"content_index":0}"#
)]
#[case::audio_done_gets_both(
    r#"{"type":"response.output_audio.done"}"#,
    r#"{"type":"response.output_audio.done","output_index":0,"content_index":0}"#
)]
#[case::explicit_null_is_kept(
    r#"{"type":"response.output_text.delta","output_index":null,"content_index":3,"delta":"hi"}"#,
    r#"{"type":"response.output_text.delta","output_index":null,"content_index":3,"delta":"hi"}"#
)]
#[case::other_events_are_untouched(
    r#"{"type":"response.created","response":{}}"#,
    r#"{"type":"response.created","response":{}}"#
)]
fn injects_only_the_missing_source_indices(#[case] input: &str, #[case] expected: &str) {
    let mut normalizer = XaiRealtimeNormalizer::default();
    assert_eq!(normalized(&mut normalizer, input), json(expected));
}

#[rstest]
#[case::top_level_missing(
    r#"{"turn_detection":{"type":"server_vad"}}"#,
    r#"{"turn_detection":{"type":"server_vad","create_response":true}}"#
)]
#[case::top_level_false_survives(
    r#"{"turn_detection":{"type":"server_vad","create_response":false}}"#,
    r#"{"turn_detection":{"type":"server_vad","create_response":false}}"#
)]
#[case::top_level_null_survives(
    r#"{"turn_detection":{"type":"server_vad","create_response":null}}"#,
    r#"{"turn_detection":{"type":"server_vad","create_response":null}}"#
)]
#[case::nested_missing(
    r#"{"audio":{"input":{"turn_detection":{"type":"server_vad"}}}}"#,
    r#"{"audio":{"input":{"turn_detection":{"type":"server_vad","create_response":true}}}}"#
)]
#[case::nested_false_survives(
    r#"{"audio":{"input":{"turn_detection":{"type":"server_vad","create_response":false}}}}"#,
    r#"{"audio":{"input":{"turn_detection":{"type":"server_vad","create_response":false}}}}"#
)]
#[case::non_server_vad_is_untouched(
    r#"{"turn_detection":{"type":"semantic_vad"}}"#,
    r#"{"turn_detection":{"type":"semantic_vad"}}"#
)]
#[case::unknown_kind_survives(
    r#"{"turn_detection":{"type":null}}"#,
    r#"{"turn_detection":{"type":null}}"#
)]
#[case::audio_not_an_object(r#"{"audio":"off"}"#, r#"{"audio":"off"}"#)]
#[case::without_turn_detection(r#"{"model":"grok-voice"}"#, r#"{"model":"grok-voice"}"#)]
fn patches_create_response_only_when_absent(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(patch_outgoing_session(json(input)), json(expected));
}

#[test]
fn created_usage_becomes_null() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let empty = normalized(
        &mut normalizer,
        r#"{"type":"response.created","response":{"id":"r","usage":{}}}"#,
    );
    assert_eq!(
        empty,
        json(r#"{"type":"response.created","response":{"id":"r","usage":null}}"#)
    );
    let null = normalized(
        &mut normalizer,
        r#"{"type":"response.created","response":{"usage":null}}"#,
    );
    assert_eq!(
        null,
        json(r#"{"type":"response.created","response":{"usage":null}}"#)
    );
    let scalar = normalized(
        &mut normalizer,
        r#"{"type":"response.created","response":{"usage":7}}"#,
    );
    assert_eq!(
        scalar,
        json(r#"{"type":"response.created","response":{"usage":null}}"#)
    );
}

#[test]
fn done_usage_becomes_the_zero_record() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let empty = normalized(
        &mut normalizer,
        r#"{"type":"response.done","response":{"usage":{}}}"#,
    );
    assert_eq!(
        empty,
        json(
            r#"{"type":"response.done","response":{"usage":{"total_tokens":0,"input_tokens":0,"output_tokens":0,"input_token_details":{"cached_tokens":0,"text_tokens":0,"audio_tokens":0},"output_token_details":{"cached_tokens":0,"text_tokens":0,"audio_tokens":0}}}}"#
        )
    );
    let scalar = normalized(
        &mut normalizer,
        r#"{"type":"response.done","response":{"usage":"bad"}}"#,
    );
    assert_eq!(
        scalar,
        json(r#"{"type":"response.done","response":{"usage":null}}"#)
    );
}

#[test]
fn usage_stays_untouched_when_it_cannot_be_normalized() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let missing = normalized(&mut normalizer, r#"{"type":"response.done","response":{}}"#);
    assert_eq!(missing, json(r#"{"type":"response.done","response":{}}"#));
    let no_response = normalized(&mut normalizer, r#"{"type":"response.done","usage":{}}"#);
    assert_eq!(no_response, json(r#"{"type":"response.done","usage":{}}"#));
    let other_event = normalized(
        &mut normalizer,
        r#"{"type":"response.in_progress","response":{"usage":{}}}"#,
    );
    assert_eq!(
        other_event,
        json(r#"{"type":"response.in_progress","response":{"usage":{}}}"#)
    );
}

#[test]
fn usage_defaults_fill_missing_token_details() {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let created = normalized(
        &mut normalizer,
        r#"{"type":"response.created","response":{"usage":{"total_tokens":9,"input_tokens":4,"output_tokens":5,"input_token_details":{"cached_tokens":2,"reasoning_tokens":1}}}}"#,
    );
    assert_eq!(
        created,
        json(
            r#"{"type":"response.created","response":{"usage":{"total_tokens":9,"input_tokens":4,"output_tokens":5,"input_token_details":{"cached_tokens":2,"text_tokens":0,"audio_tokens":0,"reasoning_tokens":1},"output_token_details":{"cached_tokens":0,"text_tokens":0,"audio_tokens":0}}}}"#
        )
    );
    let null_token = normalized(
        &mut normalizer,
        r#"{"type":"response.done","response":{"usage":{"total_tokens":null,"input_token_details":null}}}"#,
    );
    assert_eq!(
        null_token,
        json(
            r#"{"type":"response.done","response":{"usage":{"total_tokens":null,"input_tokens":0,"output_tokens":0,"input_token_details":{"cached_tokens":0,"text_tokens":0,"audio_tokens":0},"output_token_details":{"cached_tokens":0,"text_tokens":0,"audio_tokens":0}}}}"#
        )
    );
}

#[test]
fn connection_uses_the_xai_origin_and_authorization_only() {
    let (connection, logger) = prepared(None, None);
    assert_eq!(XAI_API_BASE, "https://api.x.ai/v1");
    assert_eq!(
        connection.url,
        "wss://api.x.ai/v1/realtime?model=grok-4-1-fast-non-reasoning"
    );
    assert_eq!(
        connection.headers,
        Headers::from([(String::from("Authorization"), format!("Bearer {KEY}"))])
    );
    assert_eq!(connection.ssl, RealtimeSsl::Shared);
    assert_eq!(connection.max_message_size_bytes, MAX_SIZE);
    assert!(connection.force_transcription_model.is_none());
    assert_eq!(logger.calls, vec![connection.url.clone()]);
}

#[test]
fn connection_honours_an_explicit_base_and_the_intent() {
    let query: QueryParams =
        QueryParams::from([(String::from("intent"), Some(String::from("transcription")))]);
    let (connection, _logger) = prepared(Some("https://gateway.test/v9"), Some(&query));
    assert_eq!(
        connection.url,
        "wss://gateway.test/v1/realtime?intent=transcription"
    );
    assert_eq!(connection.force_transcription_model.as_deref(), Some(MODEL));
}

#[test]
fn connection_requires_an_api_key() {
    let verification = Verification::Enabled;
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: None,
        model: MODEL,
        query_params: None,
        beta_realtime: false,
        ssl_verification: &verification,
        max_message_size_bytes: MAX_SIZE,
    };
    let error = prepare_connection(&settings, &mut RecordingLogger::default())
        .expect_err("a missing api key fails");
    assert!(matches!(error, RealtimeError::MissingApiKey));
}
