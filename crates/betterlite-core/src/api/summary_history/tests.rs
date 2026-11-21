#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::runtime::runtime_counter;
use futures_executor::block_on;
use rstest::rstest;
use thiserror::Error;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::boundary::{slice_around_compaction_block, strip_compaction_blocks};
use super::client_history::client_compaction_history;
use super::count::{
    CountedInput, HistoryTokenCounter, TokenComponents, TokenizerHistoryCounter,
    count_effective_tokens,
};
use super::error::SummaryHistoryError;
use super::last_question::select_last_user_question;
use super::metadata::{PROPAGATED_METADATA_KEYS, propagate_metadata};
use super::prompt::{build_summary_prompt, extract_summary_text};
use super::summary_messages::{append_text_to_content, build_summary_messages};
use super::system::{
    SystemPrompt, augment_system_with_summary, system_to_openai_message, system_to_text,
};
use super::translate::{AdapterTranslator, HistoryTranslator};
use super::trigger::resolve_trigger_tokens;
use crate::api::messages_context_clear::constants::{
    COMPACT_DEFAULT_INSTRUCTIONS, COMPACT_NO_TOOL_CALLS_SUFFIX,
};
use crate::api::messages_context_clear::result::CompactionBlock;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn list(text: &str) -> Vec<Json> {
    serde_json::from_str(text).expect("fixture literal is a JSON list")
}

fn text(value: &Json) -> String {
    serde_json::to_string(value).expect("fixture JSON encodes")
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut line = Line::default();
        event.record(&mut line);
        let entry = format!("{} {}", event.metadata().level(), line.0);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

fn logged<Output, Call>(call: Call) -> (Output, Vec<String>)
where
    Call: FnOnce() -> Output,
{
    let logs = Logs::default();
    let value = with_default(registry().with(logs.clone()), call);
    let lines = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, lines)
}

#[derive(Debug, Error)]
#[error("broken")]
struct Broken;

struct FakeTranslator {
    broken_messages: bool,
    broken_tools: bool,
}

impl HistoryTranslator for FakeTranslator {
    type Error = Broken;

    fn messages(
        &self,
        messages: &[Json],
    ) -> impl Future<Output = Result<Vec<Json>, Broken>> + Send {
        ready(if self.broken_messages {
            Err(Broken)
        } else {
            Ok(messages.to_vec())
        })
    }

    fn tools(&self, tools: &[Json]) -> Result<Vec<Json>, Broken> {
        if self.broken_tools {
            return Err(Broken);
        }
        Ok(tools
            .iter()
            .map(|tool| json(&format!(r#"{{"function":{}}}"#, text(tool))))
            .collect())
    }
}

#[derive(Default)]
struct ScriptedCounter {
    counts: Mutex<Vec<usize>>,
    calls: Mutex<Vec<String>>,
}

impl HistoryTokenCounter for ScriptedCounter {
    fn count(
        &self,
        model: &str,
        input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send {
        let call = match input {
            CountedInput::Conversation { messages, tools } => {
                let declared = tools.map(|raw| text(&Json::List(raw.to_vec())));
                format!(
                    "{model} messages={} tools={declared:?}",
                    text(&Json::List(messages.to_vec()))
                )
            }
            CountedInput::Text(counted) => format!("{model} text={counted}"),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let next = self
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        ready(next.ok_or(SummaryHistoryError::TokenCount(CountError::Overflow)))
    }
}

const BOUNDARY: &str = r#"[
    {"role":"assistant","content":[{"type":"compaction","content":"old"}]},
    {"role":"user","content":[{"type":"text","text":"a"},{"type":"compaction","content":"first"},
        {"type":"text","text":"b"},{"type":"compaction","content":"latest"},{"type":"text","text":"c"}],"id":"m"},
    {"role":"assistant","content":"tail"},
    {"role":"user","content":[]}
]"#;

#[test]
fn latest_compaction_boundary() {
    let messages = list(BOUNDARY);
    let slice = slice_around_compaction_block(&messages).expect("a compaction block");
    let kept = r#"[{"role":"user","content":[{"type":"compaction","content":"latest"},{"type":"text","text":"c"}],"id":"m"},{"role":"assistant","content":"tail"},{"role":"user","content":[]}]"#;
    assert_eq!(text(&Json::List(slice.messages.clone())), kept);
    assert_eq!(
        text(&Json::Object(slice.block)),
        r#"{"type":"compaction","content":"latest"}"#
    );
    let stripped = r#"[{"role":"user","content":[{"type":"text","text":"c"}],"id":"m"},{"role":"assistant","content":"tail"}]"#;
    assert_eq!(
        text(&Json::List(strip_compaction_blocks(&slice.messages))),
        stripped
    );
    let plain = list(
        r#"[{"role":"user","content":[{"type":"text","text":"q"}]},{"role":"user","content":null}]"#,
    );
    assert!(slice_around_compaction_block(&plain).is_none());
    assert_eq!(strip_compaction_blocks(&plain), plain);
}

#[rstest]
#[case::null_system(None, r#""Previous conversation summary: S""#)]
#[case::string_system(Some(r#""X""#), r#""Previous conversation summary: S\n\nX""#)]
#[case::first_text_block(
    Some(r#"[{"type":"image"},{"type":"text","cache_control":{"type":"ephemeral"}},{"type":"text","text":"Y"}]"#),
    r#"[{"type":"image"},{"type":"text","cache_control":{"type":"ephemeral"},"text":"Previous conversation summary: S\n\n"},{"type":"text","text":"Y"}]"#
)]
#[case::no_text_block(
    Some(r#"[{"type":"image"}]"#),
    r#"[{"type":"text","text":"Previous conversation summary: S"},{"type":"image"}]"#
)]
fn summary_system_shapes(#[case] system: Option<&str>, #[case] expected: &str) {
    let parsed: Option<SystemPrompt> = system.map(|raw| serde_json::from_str(raw).expect("system"));
    let augmented = augment_system_with_summary(parsed.as_ref(), "S");
    assert_eq!(
        serde_json::to_string(&augmented).expect("system encodes"),
        expected
    );
    let blocks = SystemPrompt::Blocks(list(
        r#"[{"type":"text","text":"a"},{"type":"text","text":""},{"type":"image","text":"x"},{"type":"text","text":5},{"type":"text","text":"b"}]"#,
    ));
    assert_eq!(system_to_text(Some(&blocks)), "a\nb");
    assert_eq!(system_to_text(None), "");
    let message = system_to_openai_message(Some(&blocks)).expect("text blocks");
    assert_eq!(text(&message), r#"{"role":"system","content":"a\n\nb"}"#);
    assert!(system_to_openai_message(Some(&SystemPrompt::Blocks(Vec::new()))).is_none());
    assert!(system_to_openai_message(Some(&SystemPrompt::Text(String::new()))).is_none());
    assert!(system_to_openai_message(None).is_none());
}

#[rstest]
#[case::missing(None, "150000 []")]
#[case::null(Some("null"), "150000 []")]
#[case::false_trigger(Some("false"), "150000 []")]
#[case::empty(Some("{}"), "150000 []")]
#[case::not_a_dict(Some(r#""yes""#), r#"150000 ["trigger_not_a_dict_using_default"]"#)]
#[case::tool_uses(
    Some(r#"{"type":"tool_uses","value":60000}"#),
    r#"60000 ["unsupported_trigger_type_tool_uses_using_input_tokens"]"#
)]
#[case::null_type(Some(r#"{"type":null,"value":null}"#), "150000 []")]
#[case::numeric_type(
    Some(r#"{"type":5}"#),
    r#"150000 ["unsupported_trigger_type_using_input_tokens"]"#
)]
#[case::text_value(
    Some(r#"{"value":"60000"}"#),
    r#"150000 ["trigger_value_not_int_using_default"]"#
)]
#[case::float_value(
    Some(r#"{"value":60000.5}"#),
    r#"150000 ["trigger_value_not_int_using_default"]"#
)]
#[case::list_value(
    Some(r#"{"value":[]}"#),
    r#"150000 ["trigger_value_not_int_using_default"]"#
)]
#[case::both_warnings(
    Some(r#"{"type":"tool_uses","value":"x"}"#),
    r#"150000 ["unsupported_trigger_type_tool_uses_using_input_tokens", "trigger_value_not_int_using_default"]"#
)]
#[case::at_minimum(Some(r#"{"type":"input_tokens","value":50000}"#), "50000 []")]
#[case::beyond_u64(
    Some(r#"{"value":100000000000000000000000}"#),
    "18446744073709551615 []"
)]
#[case::below_minimum(Some(r#"{"value":49999}"#), "error")]
#[case::negative(Some(r#"{"value":-60000}"#), "error")]
#[case::true_value(Some(r#"{"value":true}"#), "error")]
#[case::false_value(Some(r#"{"value":false}"#), "error")]
fn compaction_trigger_validation(#[case] trigger: Option<&str>, #[case] expected: &str) {
    let parsed = trigger.map(json);
    match resolve_trigger_tokens(parsed.as_ref()) {
        Ok(resolved) => assert_eq!(
            format!("{} {:?}", resolved.tokens, resolved.warnings),
            expected
        ),
        Err(error) => {
            assert_eq!(expected, "error");
            assert_eq!(error.status_code, 400);
            let message =
                "context_management.compact_20260112.trigger.value must be at least 50000 tokens";
            assert_eq!(error.message, message);
        }
    }
}

#[rstest]
#[case::custom_kept(Some(r#""  summarize  ""#), Some(r#"[{"name":"t"}]"#), "  summarize  ")]
#[case::missing(None, None, COMPACT_DEFAULT_INSTRUCTIONS)]
#[case::not_a_string(Some("5"), Some("[]"), COMPACT_DEFAULT_INSTRUCTIONS)]
#[case::blank_with_tools(Some(r#"" \n ""#), Some(r#"[{"name":"t"}]"#), "tools")]
fn summary_instruction_selection(
    #[case] instructions: Option<&str>,
    #[case] tools: Option<&str>,
    #[case] expected: &str,
) {
    let parsed = instructions.map(json);
    let declared = tools.map(list);
    let prompt = build_summary_prompt(parsed.as_ref(), declared.as_deref());
    let suffixed = format!("{COMPACT_DEFAULT_INSTRUCTIONS}{COMPACT_NO_TOOL_CALLS_SUFFIX}");
    assert_eq!(
        prompt,
        if expected == "tools" {
            suffixed.as_str()
        } else {
            expected
        }
    );
    let extract = |raw: Option<&str>| extract_summary_text(raw).expect("pattern compiles");
    assert_eq!(
        extract(Some("x <SUMMARY> A\nB </SUMMARY> <summary>two</summary>")).as_deref(),
        Some("A\nB")
    );
    assert_eq!(
        extract(Some("<Summary>one</summary>")).as_deref(),
        Some("one")
    );
    assert_eq!(
        extract(Some("<summary> \n </summary><summary>later</summary>")),
        None
    );
    assert_eq!(extract(Some("no tags <summary>open")), None);
    assert_eq!(extract(Some("")), None);
    assert_eq!(extract(None), None);
}

const PARENT_METADATA: &str = r#"{
    "client_key": "drop",
    "allowed_model_region": "eu",
    "litellm_parent_otel_span": "litellm_parent_otel_span",
    "litellm_call_id": "litellm_call_id",
    "user_api_key_end_user_model_max_budget": "user_api_key_end_user_model_max_budget",
    "user_api_key_user_model_max_budget": "user_api_key_user_model_max_budget",
    "user_api_key_team_model_max_budget": "user_api_key_team_model_max_budget",
    "user_api_key_model_max_budget": "user_api_key_model_max_budget",
    "user_api_end_user_max_budget": "user_api_end_user_max_budget",
    "user_api_key_end_user_id": "user_api_key_end_user_id",
    "user_api_key_project_id": "user_api_key_project_id",
    "user_api_key_org_id": "user_api_key_org_id",
    "user_api_key_user_email": "user_api_key_user_email",
    "user_api_key_user_id": "user_api_key_user_id",
    "user_api_key_team_alias": "user_api_key_team_alias",
    "user_api_key_team_id": "user_api_key_team_id",
    "user_api_key_alias": null,
    "user_api_key": "user_api_key"
}"#;

#[test]
fn summary_metadata_allowlist() {
    let parent: Object = serde_json::from_str(PARENT_METADATA).expect("parent metadata");
    let propagated = propagate_metadata(Some(&parent));
    assert_eq!(
        propagated.keys().map(String::as_str).collect::<Vec<_>>(),
        PROPAGATED_METADATA_KEYS
    );
    assert_eq!(propagated.get("user_api_key_alias"), Some(&Json::Null));
    assert_eq!(
        propagated.get("litellm_call_id"),
        Some(&Json::Text("litellm_call_id".to_owned()))
    );
    assert!(propagate_metadata(Some(&Object::new())).is_empty());
    assert!(propagate_metadata(None).is_empty());
}

const COUNTED: &str = r#"[{"role":"user","content":[{"type":"compaction","content":"x"},{"type":"text","text":"hi"}]}]"#;

fn counted<Translator>(
    translator: &Translator,
    block: Option<&str>,
    tools: Option<&str>,
    counts: &[usize],
) -> (Result<usize, SummaryHistoryError>, Vec<String>, Vec<String>)
where
    Translator: HistoryTranslator + Sync,
{
    let counter = ScriptedCounter {
        counts: Mutex::new(counts.iter().rev().copied().collect()),
        calls: Mutex::default(),
    };
    let messages = list(COUNTED);
    let compaction: Option<CompactionBlock> =
        block.map(|raw| serde_json::from_str(raw).expect("block"));
    let declared = tools.map(list);
    let system = SystemPrompt::Blocks(list(
        r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#,
    ));
    let components = TokenComponents {
        model: "gpt-4o",
        messages: &messages,
        compaction_block: compaction.as_ref(),
        tools: declared.as_deref(),
        system: Some(&system),
    };
    let (total, logs) =
        logged(|| block_on(count_effective_tokens(components, translator, &counter)));
    (
        total,
        counter
            .calls
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
        logs,
    )
}

#[test]
fn effective_token_components() {
    let fake = FakeTranslator {
        broken_messages: false,
        broken_tools: false,
    };
    let block = Some(r#"{"type":"compaction","content":"sum"}"#);
    let (total, calls, logs) = counted(&fake, block, Some(r#"[{"name":"t"}]"#), &[10, 3, 4]);
    assert_eq!(total.expect("counted"), 17);
    let messages = r#"gpt-4o messages=[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#;
    let expected = [
        format!(r#"{messages} tools=Some("[{{\"function\":{{\"name\":\"t\"}}}}]")"#),
        "gpt-4o text=sum".to_owned(),
        "gpt-4o text=a\nb".to_owned(),
    ];
    assert_eq!(calls, expected);
    assert!(logs.is_empty());
    let (empty, empty_calls, _) = counted(
        &fake,
        Some(r#"{"type":"compaction","content":""}"#),
        Some("[]"),
        &[10, 4],
    );
    assert_eq!(empty.expect("counted"), 14);
    assert_eq!(
        empty_calls.first().map(String::as_str),
        Some(format!("{messages} tools=None").as_str())
    );
    let (failed, _, _) = counted(&fake, None, None, &[10]);
    assert!(matches!(
        failed,
        Err(SummaryHistoryError::TokenCount(CountError::Overflow))
    ));
    let broken = FakeTranslator {
        broken_messages: true,
        broken_tools: true,
    };
    let (raw, raw_calls, raw_logs) = counted(&broken, None, Some(r#"[{"name":"t"}]"#), &[10, 4]);
    assert_eq!(raw.expect("counted"), 14);
    assert_eq!(
        raw_calls.first().map(String::as_str),
        Some(format!(r#"{messages} tools=Some("[{{\"name\":\"t\"}}]")"#).as_str())
    );
    let fallbacks = [
        "DEBUG compact_20260112: anthropic→openai translation failed during token count, falling back to raw messages: broken",
        "DEBUG compact_20260112: anthropic→openai tools translation failed during token count, falling back to raw tools: broken",
    ];
    assert_eq!(raw_logs, fallbacks);
    let tools = r#"[{"name":"t","input_schema":{"type":"object"}}]"#;
    let (real, real_calls, _) = counted(&AdapterTranslator, None, Some(tools), &[10, 4]);
    assert_eq!(real.expect("counted"), 14);
    assert!(real_calls.first().is_some_and(
        |call| call.contains(r#"\"type\":\"function\""#) && call.contains(r#""text":"hi""#)
    ));
}

#[rstest]
#[case::missing(None, r#""P""#)]
#[case::null(Some("null"), r#""P""#)]
#[case::empty(Some(r#""""#), r#""P""#)]
#[case::text(Some(r#""Q""#), r#""Q\n\nP""#)]
#[case::parts(
    Some(r#"[{"type":"image"}]"#),
    r#"[{"type":"image"},{"type":"text","text":"P"}]"#
)]
#[case::scalar(Some("7"), r#"[7,{"type":"text","text":"P"}]"#)]
fn summary_messages_merge_user(#[case] content: Option<&str>, #[case] expected: &str) {
    let parsed = content.map(json);
    assert_eq!(
        text(&append_text_to_content(parsed.as_ref(), "P")),
        expected
    );
    let system = SystemPrompt::Text("SYS".to_owned());
    let history = list(
        r#"[{"role":"user","content":"Q"},{"role":"system","content":"note"},{"role":"user","content":[{"type":"compaction"},{"type":"text","text":"R"}]}]"#,
    );
    let real = block_on(build_summary_messages(
        &history,
        "P",
        Some(&system),
        &AdapterTranslator,
    ));
    let merged = r#"[{"role":"system","content":"SYS"},{"role":"user","content":"Q"},{"role":"system","content":"note"},{"role":"user","content":[{"type":"text","text":"R"},{"type":"text","text":"P"}]}]"#;
    assert_eq!(text(&Json::List(real)), merged);
    let broken = FakeTranslator {
        broken_messages: true,
        broken_tools: false,
    };
    let assistant_last = list(
        r#"[{"role":"user","content":[{"type":"compaction"}]},{"role":"assistant","content":"A"}]"#,
    );
    let (raw, logs) = logged(|| {
        block_on(build_summary_messages(
            &assistant_last,
            "P",
            Some(&system),
            &broken,
        ))
    });
    assert_eq!(
        text(&Json::List(raw)),
        r#"[{"role":"system","content":"SYS"},{"role":"assistant","content":"A"},{"role":"user","content":"P"}]"#
    );
    let warning = "WARN compact_20260112: anthropic→openai translation failed when building summary call; falling back to raw shape: broken";
    assert_eq!(logs, [warning]);
    let fresh = block_on(build_summary_messages(&[], "P", None, &broken));
    assert_eq!(
        text(&Json::List(fresh)),
        r#"[{"role":"user","content":"P"}]"#
    );
}

#[rstest]
#[case::skips_tool_results_and_empty(
    r#"[{"role":"user","content":"first"},{"role":"assistant","content":"a"},{"role":"user","content":[{"type":"tool_result"}]},{"role":"user","content":[]}]"#,
    r#"{"role":"user","content":"first"}"#
)]
#[case::strips_mixed(
    r#"[{"role":"user","content":[{"type":"image"},{"type":"tool_result"},{"type":"text","text":"q"},{"type":"mystery"}],"name":"n"}]"#,
    r#"{"role":"user","content":[{"type":"image"},{"type":"text","text":"q"},{"type":"mystery"}],"name":"n"}"#
)]
#[case::null_content(
    r#"[{"role":"user","content":null}]"#,
    r#"{"role":"user","content":null}"#
)]
#[case::empty_content(r#"[{"role":"user","content":""}]"#, r#"{"role":"user","content":""}"#)]
#[case::no_user(
    r#"[{"role":"assistant","content":"a"}]"#,
    r#"{"role":"user","content":"Please continue based on the conversation summary above."}"#
)]
fn last_real_user_question(#[case] messages: &str, #[case] expected: &str) {
    assert_eq!(text(&select_last_user_question(&list(messages))), expected);
}

#[rstest]
#[case::summary_and_tail(
    r#"[{"role":"user","content":"old"},{"role":"assistant","content":[{"type":"compaction","content":"S"}]},{"role":"user","content":"next"}]"#,
    r#"[{"role":"user","content":"next"}]"#,
    r#""Previous conversation summary: S\n\nX""#,
    true
)]
#[case::empty_summary(
    r#"[{"role":"assistant","content":[{"type":"compaction","content":""}]},{"role":"user","content":"next"}]"#,
    r#"[{"role":"user","content":"next"}]"#,
    r#""X""#,
    false
)]
#[case::nothing_left(
    r#"[{"role":"assistant","content":[{"type":"compaction","content":5}]}]"#,
    r#"[{"role":"user","content":"Please continue based on the conversation summary above."}]"#,
    r#""X""#,
    false
)]
fn client_compaction_without_edit(
    #[case] messages: &str,
    #[case] downstream: &str,
    #[case] system: &str,
    #[case] summary_logged: bool,
) {
    let original = SystemPrompt::Text("X".to_owned());
    let (result, logs) = logged(|| client_compaction_history(&list(messages), Some(&original)));
    let applied = result.expect("re-encodes").expect("a compaction block");
    let expected: Vec<DecisionJson> = serde_json::from_str(downstream).expect("downstream");
    assert_eq!(applied.messages, expected);
    let expected_system: DecisionJson = serde_json::from_str(system).expect("system");
    assert_eq!(applied.system, Some(expected_system));
    assert!(
        applied.applied_edits.is_empty()
            && applied.compaction_block.is_none()
            && applied.iterations_usage.is_none()
    );
    let mut lines = vec!["INFO compact_20260112: client compaction block in message history; applying slice-only forwarding (no context_management edit)".to_owned()];
    lines.extend(summary_logged.then(|| {
        "INFO compact_20260112: compaction summary added to main call system prefix (1 chars)"
            .to_owned()
    }));
    assert_eq!(logs, lines);
    let plain = list(r#"[{"role":"user","content":"q"}]"#);
    assert!(
        client_compaction_history(&plain, None)
            .expect("re-encodes")
            .is_none()
    );
}

#[test]
fn tokenizer_counter_counts_text_and_turns() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  master_key: test-master\n")
    .expect("config");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy");
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).expect("client"));
    let catalog = BundledCatalog::default().get();
    let counter = TokenizerHistoryCounter {
        counter: Arc::new(runtime_counter(&config, &client, &catalog).expect("counter")),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let words = runtime
        .block_on(counter.count("gpt-3.5-turbo", CountedInput::Text("hello world")))
        .expect("text counts");
    assert_eq!(words, 2);
    let turns = list(r#"[{"role":"user","content":"hello world"}]"#);
    let conversation = CountedInput::Conversation {
        messages: &turns,
        tools: None,
    };
    let counted = runtime
        .block_on(counter.count("gpt-3.5-turbo", conversation))
        .expect("turns count");
    assert!(counted > words);
}
