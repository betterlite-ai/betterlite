#![cfg(test)]

mod cases;
mod corpus;

use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;

use crate::error::OpenAiError;
use crate::response_events::{RawResponse, ResponsesReply, transform_response};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

fn compact_call(
    text: &str,
    status: u16,
    headers: &IndexMap<String, String>,
    now: i64,
) -> (Result<ResponsesReply, OpenAiError>, LoggingState) {
    let redactor = redactor();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339(NOW).unwrap(),
    };
    let reply = RawResponse {
        status,
        headers,
        text,
    };
    let result = transform_response(&reply, now, &mut state, &env);
    (result, state)
}

fn header_map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    let mut headers = IndexMap::new();
    for (name, value) in pairs {
        headers.insert((*name).to_owned(), (*value).to_owned());
    }
    headers
}

#[derive(Clone, Copy)]
enum Scenario {
    EncodedResponseId,
    DeleteResult,
    GetStrictResponse,
    ListDefaultLimitOrder,
    IncludeCommaQuery,
    CancelResponse,
    CompactKeepsQuery,
    CompactReplayInput,
    CompactLaxResponse,
    ResourceJsonErrorStatus,
}

#[rstest]
#[case::encoded_response_id(Scenario::EncodedResponseId)]
#[case::delete_result(Scenario::DeleteResult)]
#[case::get_strict_response(Scenario::GetStrictResponse)]
#[case::list_default_limit_order(Scenario::ListDefaultLimitOrder)]
#[case::include_comma_query(Scenario::IncludeCommaQuery)]
#[case::cancel_response(Scenario::CancelResponse)]
#[case::compact_keeps_query(Scenario::CompactKeepsQuery)]
#[case::compact_replay_input(Scenario::CompactReplayInput)]
#[case::compact_lax_response(Scenario::CompactLaxResponse)]
#[case::resource_json_error_status(Scenario::ResourceJsonErrorStatus)]
fn response_resources_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::EncodedResponseId => cases::encoded_response_id(),
        Scenario::DeleteResult => cases::delete_result(),
        Scenario::GetStrictResponse => cases::get_strict_response(),
        Scenario::ListDefaultLimitOrder => cases::list_default_limit_order(),
        Scenario::IncludeCommaQuery => cases::include_comma_query(),
        Scenario::CancelResponse => cases::cancel_response(),
        Scenario::CompactKeepsQuery => cases::compact_keeps_query(),
        Scenario::CompactReplayInput => cases::compact_replay_input(),
        Scenario::CompactLaxResponse => cases::compact_lax_response(),
        Scenario::ResourceJsonErrorStatus => cases::resource_json_error_status(),
    }
}
