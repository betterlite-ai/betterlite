#![cfg(test)]

use std::future::Future;
use std::thread;

use betterlite_context::chat_config_selection::tags::{
    ChatConfigTag, ConfigConstruction, ConfigSelection,
};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use tokio::runtime::Builder;

use super::super::embedding::{EmbeddingCall, embed};
use super::super::{dispatch, predibase, replicate};
use super::support::{
    NOW_SECONDS, Services, chat_context, empty_globals, environment, http_reply, object,
    replicate_reply, user_message,
};

const MODEL: &str = "meta-llama/llama-2-7b-chat";
const BASE: Option<&str> = Some("http://mock");
const GENERATED: &str = r#"{"generated_text":"hello"}"#;
const REPLICATE_MODEL: &str = "meta-llama/llama-2-70b-chat";
const CREATE: &str = r#"{"urls":{"get":"https://poll/1"}}"#;
const DONE: &str = r#"{"status":"succeeded","output":["ok"]}"#;

// blocks on one call's future on the second thread and gives back what that thread received
fn on_second_thread<Output, Call>(future: Call) -> Output
where
    Output: Send,
    Call: Future<Output = Output> + Send,
{
    thread::scope(|scope| {
        scope
            .spawn(|| {
                Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(future)
            })
            .join()
            .unwrap()
    })
}

#[test]
fn dispatch_completes_on_a_second_thread() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let (mut state, log_env) = services.logging();
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context("m", &mut messages, &mut options, None);
    let selection = ConfigSelection {
        tag: ChatConfigTag::OpenAiGpt,
        construction: ConfigConstruction::Shared,
    };
    let mut log = LogContext {
        state: &mut state,
        env: &log_env,
    };
    let outcome = on_second_thread(dispatch(&selection, &mut call, &deps, &mut log));
    assert!(outcome.unwrap().is_none());
    assert!(services.http.sent("POST").is_empty());
}

#[test]
fn the_embedding_completes_on_a_second_thread() {
    let reply = http_reply(200, "[[0.5,0.25],[0.1,0.2]]");
    let services = Services::new(Vec::new(), vec![reply], Vec::new());
    let globals = empty_globals();
    let env = environment(&[("HUGGINGFACE_API_KEY", "hf-key")]);
    let deps = services.deps(&globals, &env);
    let (mut state, log_env) = services.logging();
    let input = vec![String::from("a"), String::from("b")];
    let options = object(&[
        (
            "input_type",
            PromptJson::Text(String::from("feature-extraction")),
        ),
        ("wait_for_model", PromptJson::Bool(true)),
    ]);
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let mut log = LogContext {
        state: &mut state,
        env: &log_env,
    };
    let response = on_second_thread(embed(&call, &deps, &mut log)).unwrap();
    assert_eq!(response.data.len(), 2);
    assert_eq!(response.usage.prompt_tokens, 2);
}

#[test]
fn the_predibase_entry_completes_on_a_second_thread() {
    let services = Services::new(Vec::new(), vec![http_reply(200, GENERATED)], Vec::new());
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let (mut state, log_env) = services.logging();
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    let mut log = LogContext {
        state: &mut state,
        env: &log_env,
    };
    let completion = on_second_thread(predibase::complete(&mut call, &deps, &mut log)).unwrap();
    let predibase::Completion::Reply(reply) = completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.model, MODEL);
    assert_eq!(reply.created, NOW_SECONDS);
    let content = reply
        .choices
        .first()
        .and_then(|choice| choice.content.as_deref());
    assert_eq!(content, Some("hello"));
}

#[test]
fn the_replicate_entry_completes_on_a_second_thread() {
    let replies = vec![replicate_reply(CREATE), replicate_reply(DONE)];
    let services = Services::new(Vec::new(), Vec::new(), replies);
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let (mut state, log_env) = services.logging();
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(REPLICATE_MODEL, &mut messages, &mut options, None);
    let mut log = LogContext {
        state: &mut state,
        env: &log_env,
    };
    let completion = on_second_thread(replicate::complete(&mut call, &deps, &mut log)).unwrap();
    let replicate::Completion::Reply(reply) = completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "ok");
    assert_eq!(reply.model, "replicate/meta-llama/llama-2-70b-chat");
    assert_eq!(reply.created, Some(NOW_SECONDS));
}
