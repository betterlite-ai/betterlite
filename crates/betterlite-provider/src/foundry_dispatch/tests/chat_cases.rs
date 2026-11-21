use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::message_content::Content;
use indexmap::IndexMap;

use super::support::{
    self, BASE, Env, KEY, REPLY, chat_call, chat_http, field, first_content, loopback,
    proxied_client, run_chat, serve_once, stream_http, text_header,
};
use crate::foundry_dispatch::chat::{self, Answer};
use crate::foundry_dispatch::fixtures::logger::RecordingLogger;
use crate::foundry_dispatch::route::{self, Route};
use crate::openai_dispatch::logging::LoggedCall;
use crate::operation::{ChatCompletion, OperationError, OperationResult};

const STREAM_REPLY: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\ndata: {\"id\":\"chatcmpl-foundry\"}\n\n";

pub(super) fn chat_case() {
    route_selection();
    deployment_key_reply();
    global_key_fallback();
    other_routes_skip_chat();
}

// main.py resolves the agents route first, then a case-insensitive claude substring, then the chat route
fn route_selection() {
    assert_eq!(route::select("agents/agent-claude"), Route::Agents);
    assert_eq!(route::select("CLAUDE-sonnet"), Route::Claude);
    assert_eq!(route::select("mistral-large"), Route::Chat);
}

fn deployment_key_reply() {
    let env = Env::with_globals("https://global.example.test", "global-key");
    let deployment = support::deployment(BASE, Some(KEY));
    let request = support::simple_request("mistral-large");
    let call = chat_call(&request, &deployment, "mistral-large", false);
    let (result, logger) = run_chat(call, &env, support::supplied(200, REPLY));
    let outcome = result.expect("the chat dispatch runs");
    assert!(logger.calls.is_empty());
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) = outcome.result else {
        panic!("the chat dispatch returns a chat reply");
    };
    assert_eq!(completion.model, "azure_ai/provider-model");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("hello")
    );
    let http = chat_http(&outcome.prepared);
    assert_eq!(
        http.url.as_str(),
        "https://res.services.ai.azure.com/models/chat/completions"
    );
    // a Foundry host takes the key in api-key, as LiteLLM's azure v1 host rule says
    assert_eq!(
        text_header(&http.headers, "api-key"),
        Some("sk-foundry-fixture")
    );
    let sent: JsonValue = serde_json::from_slice(&http.body).expect("the request body");
    assert_eq!(
        field(&sent, "model"),
        &JsonValue::Text(String::from("mistral-large"))
    );
    assert_eq!(first_content(&sent), &JsonValue::Text(String::from("hi")));
}

// the same context with no deployment key uses the global one through B549
fn global_key_fallback() {
    let env = Env::with_globals("https://global.example.test", "global-key");
    let deployment = support::deployment(BASE, None);
    let request = support::simple_request("mistral-large");
    let call = chat_call(&request, &deployment, "mistral-large", false);
    let (result, _logger) = run_chat(call, &env, support::supplied(200, REPLY));
    let outcome = result.expect("the chat dispatch runs");
    assert_eq!(
        text_header(&chat_http(&outcome.prepared).headers, "api-key"),
        Some("global-key")
    );
}

// neither the agents nor the claude model reaches the chat leaf
fn other_routes_skip_chat() {
    let env = Env::with_globals("https://global.example.test", "global-key");
    let deployment = support::deployment(BASE, None);
    for (model, branch) in [
        ("agents/agent-claude", "agents"),
        ("claude-sonnet", "claude"),
    ] {
        let request = support::simple_request(model);
        let call = chat_call(&request, &deployment, model, false);
        let error = chat::prepare::prepare(&call, &env.globals())
            .err()
            .expect("the route is not chat");
        assert_eq!(
            error.to_string(),
            format!("the {branch} branch of the Foundry provider is not this chat call")
        );
    }
}

pub(super) fn command_r_case() {
    command_r_rewrites_tool_content();
    command_r_match_is_case_sensitive();
    supplied_extra_headers_replace();
}

fn command_r_rewrites_tool_content() {
    let request = support::request(
        r#"{"model":"command-r-plus","messages":[{"role":"tool","content":"not json"},{"role":"user","content":"next"}]}"#,
    );
    let deployment = support::deployment(BASE, Some(KEY));
    let env = Env::default();
    let call = chat_call(&request, &deployment, "command-r-plus", false);
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the command-r plan");
    assert_eq!(plan.messages.len(), 2);
    assert_eq!(plan.messages[0].role, "tool");
    assert_eq!(
        plan.messages[0].content,
        Some(Content::Text(String::from(r#"{"result":"not json"}"#)))
    );
    assert_eq!(
        plan.messages[1].content,
        Some(Content::Text(String::from("next")))
    );
}

// Command-R keeps its messages untouched, since LiteLLM's substring check is case-sensitive
fn command_r_match_is_case_sensitive() {
    let request = support::request(
        r#"{"model":"Command-R","messages":[{"role":"tool","content":"not json"}]}"#,
    );
    let deployment = support::deployment(BASE, Some(KEY));
    let env = Env::default();
    let call = chat_call(&request, &deployment, "Command-R", false);
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the plain plan");
    assert_eq!(
        plan.messages[0].content,
        Some(Content::Text(String::from("not json")))
    );
}

// a supplied extra_headers map, even an empty one, replaces the optional map's own entry
fn supplied_extra_headers_replace() {
    let request = support::request(
        r#"{"model":"mistral-large","messages":[{"role":"user","content":"hi"}],"extra_headers":{"x-given":"1"}}"#,
    );
    let deployment = support::deployment(BASE, Some(KEY));
    let env = Env::default();
    let call = chat_call(&request, &deployment, "mistral-large", false);
    let kept = chat::prepare::prepare(&call, &env.globals()).expect("the kept plan");
    assert_eq!(
        text_header(&chat_http(&kept.prepared).headers, "x-given"),
        Some("1")
    );
    let empty = Headers::new();
    let emptied = chat::Call {
        extra_headers: Some(&empty),
        ..chat_call(&request, &deployment, "mistral-large", false)
    };
    let replaced = chat::prepare::prepare(&emptied, &env.globals()).expect("the overwritten plan");
    assert!(text_header(&chat_http(&replaced.prepared).headers, "x-given").is_none());
}

pub(super) fn post_call_case() {
    success_logs_nothing();
    stream_success_logs_report();
    failure_logs_text_and_headers();
    preparation_failure_logs_nothing();
}

// a normal non-stream success logs nothing
fn success_logs_nothing() {
    let env = Env::with_header("x-logged", "1");
    let deployment = support::deployment(BASE, Some(KEY));
    let request = support::simple_request("mistral-large");
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&request, &deployment, "mistral-large", false)
    };
    let (result, logger) = run_chat(call, &env, support::supplied(200, REPLY));
    result.expect("the chat dispatch runs");
    assert!(logger.calls.is_empty());
}

// Runs the streamed call against a loopback server and returns what the logger saw
fn streamed_logs(env: &Env) -> Vec<LoggedCall> {
    support::block_on(async {
        let (listener, api_base) = loopback().await;
        let deployment = support::deployment(&api_base, Some(KEY));
        let request = support::simple_request("mistral-large");
        let call = chat::Call {
            headers: env.headers.as_ref(),
            ..chat_call(&request, &deployment, "mistral-large", true)
        };
        let plan = chat::prepare::prepare(&call, &env.globals()).expect("the stream plan");
        let expected = stream_http(&plan.prepared).body.clone();
        let client = proxied_client();
        let mut logger = RecordingLogger::default();
        let (result, _sent) = tokio::join!(
            async {
                chat::Dispatch {
                    call,
                    globals: env.globals(),
                    answer: Answer::Client(&client),
                }
                .run(&mut logger)
                .await
            },
            serve_once(listener, &expected, STREAM_REPLY)
        );
        result.expect("the stream dispatch runs");
        logger.calls
    })
}

// a stream success logs the stream report with the resolved headers, through the real client
fn stream_success_logs_report() {
    let env = Env::with_header("x-logged", "1");
    let logged = streamed_logs(&env);
    assert_eq!(logged.len(), 1);
    assert_eq!(
        logged[0].original_response,
        Json::Object(IndexMap::from([(String::from("stream"), Json::Bool(true))]))
    );
    let headers = logged[0].headers.as_ref().expect("the headers");
    assert_eq!(
        headers.get("x-logged"),
        Some(&Json::Text(String::from("1")))
    );
}

// an execution failure logs the error text plus the headers, then rethrows
fn failure_logs_text_and_headers() {
    let env = Env::with_header("x-logged", "1");
    let deployment = support::deployment(BASE, Some(KEY));
    let request = support::simple_request("mistral-large");
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&request, &deployment, "mistral-large", false)
    };
    let (result, logger) = run_chat(call, &env, support::supplied(500, r#"{"error":"boom"}"#));
    let Err(error) = result else {
        panic!("a 500 is a failure");
    };
    assert!(matches!(error, OperationError::Provider { .. }));
    assert_eq!(logger.calls.len(), 1);
    let Json::Text(text) = &logger.calls[0].original_response else {
        panic!("the failure log carries the error text");
    };
    assert_eq!(text, &error.to_string());
    assert!(logger.calls[0].headers.is_some());
}

// a preparation failure logs nothing
fn preparation_failure_logs_nothing() {
    let env = Env::with_header("x-logged", "1");
    let deployment = support::deployment(BASE, Some(KEY));
    let request = support::simple_request("claude-sonnet");
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&request, &deployment, "claude-sonnet", false)
    };
    let (result, logger) = run_chat(call, &env, support::supplied(200, REPLY));
    let Err(_failure) = result else {
        panic!("a claude model is not a chat call");
    };
    assert!(logger.calls.is_empty());
}
