#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::iter::once;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use tokio::runtime::Builder;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::dispatch::async_post_mcp_tool_call_hook;
use super::types::{McpPostCallResponse, McpToolHookCall, McpToolResult};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};
use crate::logging_state::{LoggingInit, LoggingState};
use crate::registry::CallbackRegistry;
use crate::registry::Registration;
use crate::registry::identity::CallbackIdentity;
use crate::registry::phases::CallbackList;
use crate::registry::sink::CallbackSink;

const NON_BLOCKING: &str = "LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging";

fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn start_time() -> DateTime<FixedOffset> {
    time("2026-01-01T00:00:00+00:00")
}

fn end_time() -> DateTime<FixedOffset> {
    time("2026-01-01T00:00:01+00:00")
}

fn text_item(text: &str) -> Json {
    object(&[
        ("type", Json::Text("text".to_owned())),
        ("text", Json::Text(text.to_owned())),
    ])
}

fn object(fields: &[(&str, Json)]) -> Json {
    let mut object = IndexMap::new();
    for (name, value) in fields {
        object.insert((*name).to_owned(), value.clone());
    }
    Json::Object(object)
}

fn answer() -> Json {
    object(&[("answer", Json::Number(Number::from(42_u8)))])
}

fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).unwrap())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Behavior {
    #[default]
    Nothing,
    EditInPlace,
    Replace,
    ReplaceUnchanged,
    EditAndRewriteStructured,
    ReplaceLive,
    Fail,
    RewriteLive,
    RewriteLiveThenFail,
}

#[derive(Clone)]
struct Script {
    behavior: Behavior,
    content: Vec<Json>,
    // ast-grep-ignore: no-float-money
    response_cost: Option<f64>,
    structured_content: Option<Json>,
    aliases: Vec<&'static str>,
}

impl Script {
    fn plain(behavior: Behavior) -> Self {
        Self {
            behavior,
            content: Vec::new(),
            response_cost: None,
            structured_content: None,
            aliases: Vec::new(),
        }
    }

    fn replacing(content: &Json, response_cost: Option<f64>) -> Self {
        Self {
            behavior: Behavior::Replace,
            content: vec![content.clone()],
            response_cost,
            structured_content: None,
            aliases: Vec::new(),
        }
    }

    fn rewriting(content: &Json, structured_content: &Json) -> Self {
        Self {
            behavior: Behavior::RewriteLive,
            content: vec![content.clone()],
            response_cost: None,
            structured_content: Some(structured_content.clone()),
            aliases: Vec::new(),
        }
    }

    fn rewrite_content(&self, call: &mut McpToolHookCall<'_>) {
        call.original.content.clone_from(&self.content);
    }

    fn rewrite_structured(&self, call: &mut McpToolHookCall<'_>) {
        call.original
            .structured_content
            .clone_from(&self.structured_content);
    }

    fn edit_copy(call: &mut McpToolHookCall<'_>) {
        call.response
            .mcp_tool_call_response
            .push(text_item("edited"));
    }

    fn reply(&self, call: &McpToolHookCall<'_>) -> McpPostCallResponse {
        let mut hidden_params = call.response.hidden_params;
        if let Some(cost) = self.response_cost {
            hidden_params.response_cost = Some(cost);
        }
        McpPostCallResponse {
            mcp_tool_call_response: self.content.clone(),
            hidden_params,
        }
    }
}

struct ScriptedLogger {
    name: &'static str,
    script: Script,
    runs: Arc<Mutex<Vec<&'static str>>>,
    // ast-grep-ignore: no-float-money
    received_costs: Arc<Mutex<Vec<Option<f64>>>>,
}

impl ScriptedLogger {
    fn record(&self, cost: Option<f64>) {
        self.runs.lock().unwrap().push(self.name);
        self.received_costs.lock().unwrap().push(cost);
    }

    fn fail(&self) -> Result<Option<McpPostCallResponse>, LoggerError> {
        Err(LoggerError::Delivery {
            callback: self.name.to_owned(),
            message: "logger exploded".to_owned(),
        })
    }
}

#[async_trait]
impl CallbackLogger for ScriptedLogger {
    fn callback_name(&self) -> &str {
        self.name
    }

    async fn async_post_mcp_tool_call_hook(
        &self,
        mut call: McpToolHookCall<'_>,
    ) -> Result<Option<McpPostCallResponse>, LoggerError> {
        let script = &self.script;
        self.record(call.response.hidden_params.response_cost);
        match script.behavior {
            Behavior::Nothing => Ok(None),
            Behavior::EditInPlace => {
                Script::edit_copy(&mut call);
                Ok(None)
            }
            Behavior::Replace => Ok(Some(script.reply(&call))),
            Behavior::ReplaceUnchanged => Ok(Some(call.response.clone())),
            Behavior::EditAndRewriteStructured => {
                Script::edit_copy(&mut call);
                script.rewrite_structured(&mut call);
                Ok(None)
            }
            Behavior::ReplaceLive => {
                script.rewrite_content(&mut call);
                script.rewrite_structured(&mut call);
                Ok(Some(McpPostCallResponse {
                    mcp_tool_call_response: script.content.clone(),
                    hidden_params: call.response.hidden_params,
                }))
            }
            Behavior::Fail => self.fail(),
            Behavior::RewriteLive => {
                script.rewrite_content(&mut call);
                script.rewrite_structured(&mut call);
                Ok(None)
            }
            Behavior::RewriteLiveThenFail => {
                script.rewrite_content(&mut call);
                script.rewrite_structured(&mut call);
                self.fail()
            }
        }
    }
}

#[derive(Clone)]
struct Expected {
    content: Vec<Json>,
    structured_content: Option<Json>,
    is_error: bool,
    response_cost: Option<Json>,
    runs: Vec<&'static str>,
    // ast-grep-ignore: no-float-money
    received_costs: Vec<Option<f64>>,
    error_lines: usize,
}

#[derive(Clone)]
struct Scenario {
    result: McpToolResult,
    dynamic_success: Vec<String>,
    global_success: Vec<(&'static str, Script)>,
    expected: Expected,
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

struct Outcome {
    result: McpToolResult,
    response_cost: Option<Json>,
    runs: Vec<&'static str>,
    // ast-grep-ignore: no-float-money
    received_costs: Vec<Option<f64>>,
    error_lines: usize,
}

fn state(dynamic_success: &[String]) -> LoggingState {
    let mut context = CorrelationContext::default();
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "call_mcp_tool".to_owned(),
        start_time: start_time(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: None,
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks {
            input: None,
            success: (!dynamic_success.is_empty()).then(|| dynamic_success.to_vec()),
            async_success: None,
            failure: None,
            async_failure: None,
        },
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(16).unwrap(),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

fn original_response(result: &McpToolResult) -> Json {
    object(&[
        ("content", Json::List(result.content.clone())),
        (
            "structured_content",
            result.structured_content.clone().unwrap_or(Json::Null),
        ),
        ("is_error", Json::Bool(result.is_error)),
    ])
}

fn registry_for(
    loggers: &[(&'static str, Script)],
    runs: &Arc<Mutex<Vec<&'static str>>>,
    received_costs: &Arc<Mutex<Vec<Option<f64>>>>,
) -> CallbackRegistry {
    let mut registry = CallbackRegistry::new(16);
    for (name, script) in loggers {
        let logger: Arc<dyn CallbackLogger> = Arc::new(ScriptedLogger {
            name,
            script: script.clone(),
            runs: Arc::clone(runs),
            received_costs: Arc::clone(received_costs),
        });
        let identities = once(*name).chain(script.aliases.iter().copied());
        for class_name in identities {
            let registration = registry.register(
                CallbackIdentity {
                    class_name: class_name.to_owned(),
                    attributes: IndexMap::new(),
                },
                CallbackSink::Logger(Arc::clone(&logger)),
                CallbackList::Success,
            );
            assert_eq!(registration, Registration::Registered);
        }
    }
    registry
}

fn execute(scenario: &Scenario) -> Outcome {
    let runs = Arc::new(Mutex::new(Vec::new()));
    let received_costs = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_for(&scenario.global_success, &runs, &received_costs);
    let mut logging = state(&scenario.dynamic_success);
    logging.model_call_details.insert(
        "original_response".to_owned(),
        original_response(&scenario.result),
    );
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let runtime = Builder::new_current_thread().build().unwrap();
    let result = with_default(subscriber, || {
        runtime.block_on(async_post_mcp_tool_call_hook(
            &mut logging,
            &registry,
            scenario.result.clone(),
            start_time(),
            end_time(),
        ))
    });
    let error_lines = buffer
        .contents()
        .lines()
        .filter(|line| line.contains(NON_BLOCKING))
        .count();
    Outcome {
        result,
        response_cost: logging.model_call_details.get("response_cost").cloned(),
        runs: runs.lock().unwrap().clone(),
        received_costs: received_costs.lock().unwrap().clone(),
        error_lines,
    }
}

fn check(scenario: &Scenario, outcome: &Outcome) {
    assert_eq!(outcome.result.content, scenario.expected.content);
    assert_eq!(
        outcome.result.structured_content,
        scenario.expected.structured_content
    );
    assert_eq!(outcome.result.is_error, scenario.expected.is_error);
    assert_eq!(outcome.response_cost, scenario.expected.response_cost);
    assert_eq!(outcome.runs, scenario.expected.runs);
    assert_eq!(outcome.received_costs, scenario.expected.received_costs);
    assert_eq!(outcome.error_lines, scenario.expected.error_lines);
}

fn hello() -> McpToolResult {
    McpToolResult {
        content: vec![text_item("hello")],
        structured_content: Some(answer()),
        is_error: false,
    }
}

#[rstest]
#[case::no_loggers(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: Vec::new(),
    expected: Expected {
        content: vec![text_item("hello")],
        structured_content: Some(answer()),
        is_error: false,
        response_cost: None,
        runs: Vec::new(),
        received_costs: Vec::new(),
        error_lines: 0,
    },
})]
#[case::in_place_edit_clears_structured(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![("editor", Script::plain(Behavior::EditInPlace))],
    expected: Expected {
        content: vec![text_item("hello"), text_item("edited")],
        structured_content: None,
        is_error: true,
        response_cost: None,
        runs: vec!["editor"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::in_place_edit_with_a_live_structured_rewrite_clears_it(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![(
        "editor",
        Script {
            behavior: Behavior::EditAndRewriteStructured,
            content: Vec::new(),
            response_cost: None,
            structured_content: Some(object(&[("interim", Json::Bool(true))])),
            aliases: Vec::new(),
        },
    )],
    expected: Expected {
        content: vec![text_item("hello"), text_item("edited")],
        structured_content: None,
        is_error: true,
        response_cost: None,
        runs: vec!["editor"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::replacement_sets_cost(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![("replacer", Script::replacing(&text_item("replaced"), Some(0.25_f64)))],
    expected: Expected {
        content: vec![text_item("replaced")],
        structured_content: None,
        is_error: true,
        response_cost: Some(number(0.25)),
        runs: vec!["replacer"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::replacement_equal_content(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![("keeper", Script::plain(Behavior::ReplaceUnchanged))],
    expected: Expected {
        content: vec![text_item("hello")],
        structured_content: Some(answer()),
        is_error: false,
        response_cost: Some(Json::Null),
        runs: vec!["keeper"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::replacement_matching_the_live_content_keeps_structured(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![(
        "guardrail",
        Script {
            behavior: Behavior::ReplaceLive,
            content: vec![text_item("blocked")],
            response_cost: None,
            structured_content: Some(object(&[("blocked", Json::Bool(true))])),
            aliases: Vec::new(),
        },
    )],
    expected: Expected {
        content: vec![text_item("blocked")],
        structured_content: Some(object(&[("blocked", Json::Bool(true))])),
        is_error: false,
        response_cost: Some(Json::Null),
        runs: vec!["guardrail"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::failing_logger_continues(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![
        ("broken", Script::plain(Behavior::Fail)),
        ("replacer", Script::replacing(&text_item("replaced"), Some(0.5_f64))),
    ],
    expected: Expected {
        content: vec![text_item("replaced")],
        structured_content: None,
        is_error: true,
        response_cost: Some(number(0.5)),
        runs: vec!["broken", "replacer"],
        received_costs: vec![None, None],
        error_lines: 1,
    },
})]
#[case::live_rewrite_keeps_structured(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![(
        "guardrail",
        Script::rewriting(&text_item("rewritten"), &object(&[("blocked", Json::Bool(true))])),
    )],
    expected: Expected {
        content: vec![text_item("rewritten")],
        structured_content: Some(object(&[("blocked", Json::Bool(true))])),
        is_error: false,
        response_cost: None,
        runs: vec!["guardrail"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::failing_logger_keeps_its_live_rewrite(Scenario {
    result: hello(),
    dynamic_success: Vec::new(),
    global_success: vec![(
        "broken-guard",
        Script {
            behavior: Behavior::RewriteLiveThenFail,
            content: vec![text_item("blocked")],
            response_cost: None,
            structured_content: Some(object(&[("blocked", Json::Bool(true))])),
            aliases: Vec::new(),
        },
    )],
    expected: Expected {
        content: vec![text_item("blocked")],
        structured_content: Some(object(&[("blocked", Json::Bool(true))])),
        is_error: false,
        response_cost: None,
        runs: vec!["broken-guard"],
        received_costs: vec![None],
        error_lines: 1,
    },
})]
#[case::two_loggers_carry_hidden_params(Scenario {
    result: McpToolResult { content: vec![text_item("hello")], structured_content: None, is_error: false },
    dynamic_success: Vec::new(),
    global_success: vec![
        ("alpha", Script::replacing(&text_item("alpha"), Some(0.1_f64))),
        ("beta", Script::replacing(&text_item("beta"), None)),
    ],
    expected: Expected {
        content: vec![text_item("beta")],
        structured_content: None,
        is_error: false,
        response_cost: Some(number(0.1)),
        runs: vec!["alpha", "beta"],
        received_costs: vec![None, Some(0.1_f64)],
        error_lines: 0,
    },
})]
#[case::logger_in_both_lists_runs_once(Scenario {
    result: hello(),
    dynamic_success: vec!["guard".to_owned()],
    global_success: vec![(
        "guard",
        Script {
            behavior: Behavior::EditInPlace,
            content: Vec::new(),
            response_cost: None,
            structured_content: None,
            aliases: vec!["guard-mirror"],
        },
    )],
    expected: Expected {
        content: vec![text_item("hello"), text_item("edited")],
        structured_content: None,
        is_error: true,
        response_cost: None,
        runs: vec!["guard"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
#[case::name_entries_skipped(Scenario {
    result: hello(),
    dynamic_success: vec!["ghost".to_owned()],
    global_success: Vec::new(),
    expected: Expected {
        content: vec![text_item("hello")],
        structured_content: Some(answer()),
        is_error: false,
        response_cost: None,
        runs: Vec::new(),
        received_costs: Vec::new(),
        error_lines: 0,
    },
})]
#[case::result_without_structured_content_keeps_is_error(Scenario {
    result: McpToolResult { content: vec![text_item("hello")], structured_content: None, is_error: false },
    dynamic_success: Vec::new(),
    global_success: vec![("editor", Script::plain(Behavior::EditInPlace))],
    expected: Expected {
        content: vec![text_item("hello"), text_item("edited")],
        structured_content: None,
        is_error: false,
        response_cost: None,
        runs: vec!["editor"],
        received_costs: vec![None],
        error_lines: 0,
    },
})]
fn mcp_post_call_hook_follows_the_litellm_walk(#[case] scenario: Scenario) {
    let outcome = execute(&scenario);
    check(&scenario, &outcome);
}

struct SilentLogger {
    name: String,
}

impl CallbackLogger for SilentLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

#[tokio::test]
async fn a_logger_with_the_default_hook_leaves_the_result_unchanged() {
    let mut registry = CallbackRegistry::new(16);
    let logger = Arc::new(SilentLogger {
        name: "silent".to_owned(),
    });
    let registration = registry.register(
        CallbackIdentity {
            class_name: "silent".to_owned(),
            attributes: IndexMap::new(),
        },
        CallbackSink::Logger(logger),
        CallbackList::Success,
    );
    assert_eq!(registration, Registration::Registered);
    let mut logging = state(&[]);
    let result = McpToolResult {
        content: vec![text_item("hello")],
        structured_content: Some(answer()),
        is_error: false,
    };
    let final_result = async_post_mcp_tool_call_hook(
        &mut logging,
        &registry,
        result.clone(),
        start_time(),
        end_time(),
    )
    .await;
    assert_eq!(final_result, result);
    assert!(!logging.model_call_details.contains_key("response_cost"));
}
