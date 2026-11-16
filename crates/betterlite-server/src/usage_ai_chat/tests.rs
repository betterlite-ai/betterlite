#![cfg(test)]

mod support;

include!("tests/tools_prompt.rs");
include!("tests/fetch.rs");
include!("tests/summary.rs");
include!("tests/sse.rs");
include!("tests/source.rs");
include!("tests/flow.rs");

use std::fs;
use std::path::Path;

use betterlite_analytics::daily_metrics::MetricWithMetadata;
use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::sync::mpsc;

use crate::daily_activity::recovery::scope::ScopeIds;
use crate::usage_ai_chat::constants::{
    DEFAULT_CHAT_MODEL, ENTITY_FIELD_TAG, ENTITY_FIELD_TEAM, ENTITY_FIELD_USER, MAX_CHAT_MESSAGES,
    PAGINATED_PAGE_SIZE, TABLE_DAILY_TAG_SPEND, TABLE_DAILY_TEAM_SPEND, TABLE_DAILY_USER_SPEND,
    TOP_N_MODELS, TOP_N_PROVIDERS, USAGE_AI_TEMPERATURE, USAGE_TOP_API_KEYS_DEFAULT,
};
use crate::usage_ai_chat::error::UsageChatError;
use crate::usage_ai_chat::fetch_args::{
    FetchArguments, decode_tool_arguments, parse_csv_ids, resolve_fetch_kwargs,
};
use crate::usage_ai_chat::flow::stream_usage_ai_chat;
use crate::usage_ai_chat::messages::ChatMessage;
use crate::usage_ai_chat::model::{ModelReply, ModelToolCall};
use crate::usage_ai_chat::prompt::{SYSTEM_PROMPT_BASE, build_system_prompt};
use crate::usage_ai_chat::source::{DailyActivityUsageData, UsageDataSource as _};
use crate::usage_ai_chat::sse::{SseEvent, ToolCallStatus, sse_frame};
use crate::usage_ai_chat::summary::{EntityLabel, summarise_entity_data, summarise_usage_data};
use crate::usage_ai_chat::tools::{SchemaKind, ToolKind, UsageTool, tools_for_role};
use support::{
    ERROR_FRAME, ExtrasAbsence, RecordingRepository, SourceCall, admin_tool_reply, alias_entry,
    answer, args, breakdown, chat_parts, chat_request, chunk_frame, concrete_repository,
    dated_args, day, done_frame, drive, empty_page, entry, fake_model, fake_source, frame_tool,
    metadata, on, page, recording_database, recording_repository, require_send, spend_of,
    status_frame, tool_call,
};

const ARGS: &str = "{\"start_date\": \"2026-01-01\", \"end_date\": \"2026-01-31\"}";
const TEAM_ARGS: &str =
    "{\"start_date\":\"2026-01-01\",\"end_date\":\"2026-01-31\",\"team_ids\":\"team-1, team-2\"}";

#[test]
fn replays_the_usage_tool_arguments_corpus() {
    assert!(replay_usage_tool_arguments() > 0);
}

fn replay_usage_tool_arguments() -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/usage_tool_arguments");
    let mut count = 0_usize;
    for entry in fs::read_dir(&root).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        if let Ok(text) = String::from_utf8(bytes) {
            let _decoded = decode_tool_arguments(&text);
        }
        count = count.saturating_add(1);
    }
    count
}
