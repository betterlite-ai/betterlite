#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::hub::{Hub, HubClient, builtin_entries};
use super::{Output, PromptFactory, prompt_factory};
use crate::builtin::Message;
use crate::error::FactoryError;

const LLAMA3_RENDERED: &str = "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\ns<|eot_id|><|start_header_id|>user<|end_header_id|>\n\nu<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n";
const MISTRAL_RENDERED: &str = "<s>Error message - Conversation roles must alternate user/assistant/user/assistant/...Error message - Only user and assistant roles are supported!Error message - Conversation roles must alternate user/assistant/user/assistant/...[INST] u [/INST]";
const DEEPSEEK_RENDERED: &str = "<\u{ff5c}begin\u{2581}of\u{2581}sentence\u{ff5c}>s<\u{ff5c}User\u{ff5c}>u<\u{ff5c}Assistant\u{ff5c}><think>\n";

#[derive(Clone, Copy)]
enum Answer {
    Status(u16, &'static [u8]),
}

#[derive(Clone, Copy)]
enum Call {
    Selector,
    // a selector run against a factory whose cache has no built-in entry for the model
    Uncached,
    Template,
}

#[derive(Clone, Copy)]
enum Expect {
    Exact(&'static str),
    NoTemplate,
}

fn clock() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn messages(json_text: &str) -> Vec<Message> {
    serde_json::from_str(json_text).unwrap()
}

fn bare_messages() -> Vec<Message> {
    messages(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#)
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn hub_client(base_url: &str) -> HubClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    HubClient {
        base_url: base_url.to_owned(),
        client: ProxiedClient::outbound(&Arc::new(policy)).unwrap(),
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(request).unwrap();
        }
    }
}

fn request_path(request: &str) -> &str {
    request.split_whitespace().nth(1).unwrap()
}

async fn respond(listener: &TcpListener, answer: &Answer) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_request(&mut stream).await;
    let Answer::Status(status, body) = *answer;
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    request
}

#[rstest]
#[case::factory_default(
    "gpt2",
    None,
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "s u a"
)]
#[case::factory_builtin(
    "anthropic.claude-v2",
    Some("bedrock"),
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"
)]
#[case::factory_clarifai_claude(
    "anthropic.claude-v2",
    Some("clarifai"),
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"
)]
#[case::factory_clarifai_non_claude_falls_back_to_generic(
    "clarifai/meta-llama/some-model",
    Some("clarifai"),
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#,
    "s u"
)]
#[case::factory_anthropic_claude_2(
    "claude-2",
    Some("anthropic"),
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"
)]
#[case::factory_anthropic_claude_instant_1(
    "claude-instant-1",
    Some("anthropic"),
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"
)]
#[tokio::test]
async fn prompt_templates_prompt_template_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] wire: &str,
    #[case] expected: &str,
) {
    let output = prompt_factory(
        &factory(),
        model,
        &messages(wire),
        custom_llm_provider,
        clock(),
    )
    .await
    .unwrap();
    assert_eq!(output, Output::Text(expected.to_owned()));
}

#[tokio::test]
async fn prompt_factory_dispatches_ollama() {
    let output = prompt_factory(
        &factory(),
        "llama2",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        Some("ollama"),
        clock(),
    )
    .await
    .unwrap();
    assert!(matches!(output, Output::Ollama(_)));
}

#[rstest]
#[case::mistral("mistral-large", "mistral", "mistral_transform")]
#[case::perplexity("some-model", "perplexity", "perplexity")]
#[case::azure_text("some-model", "azure_text", "azure_text")]
#[case::watsonx("some-model", "watsonx", "watsonx")]
#[case::anthropic("claude-3-opus", "anthropic", "anthropic_messages")]
#[tokio::test]
async fn prompt_factory_delegates_installed_adapters(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] operation: &str,
) {
    let error = prompt_factory(
        &factory(),
        model,
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        Some(provider),
        clock(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FactoryError::Unsupported { operation: found } if found == operation
    ));
}

// with LiteLLM's built-in entries restored, both B177 cases render the Meta-Llama-3 template
#[rstest]
#[case::bedrock_missing_local_template_raises(
    "bedrock/meta.llama3-70b-instruct-v1:0",
    Some("bedrock")
)]
#[case::generic_missing_local_template_falls_back("meta-llama/llama-3-8b-instruct", None)]
#[tokio::test]
async fn prompt_factory_cases(#[case] model: &str, #[case] custom_llm_provider: Option<&str>) {
    let result = prompt_factory(
        &factory(),
        model,
        &bare_messages(),
        custom_llm_provider,
        clock(),
    )
    .await;
    assert_eq!(result.unwrap(), Output::Text(LLAMA3_RENDERED.to_owned()));
}

#[rstest]
#[case::builtin_mistral(Call::Template, "mistralai/Mistral-7B-Instruct-v0.1", None, vec![], Expect::Exact(MISTRAL_RENDERED))]
#[case::builtin_llama3(Call::Template, "meta-llama/Meta-Llama-3-8B-Instruct", None, vec![], Expect::Exact(LLAMA3_RENDERED))]
#[case::builtin_deepseek(Call::Template, "deepseek-r1/deepseek-r1-7b-instruct", None, vec![], Expect::Exact(DEEPSEEK_RENDERED))]
#[case::config_with_template(
    Call::Template,
    "unlisted/config-template",
    None,
    vec![Answer::Status(200, br#"{"chat_template":"{{ bos_token }}hi {{ messages[0]['content'] }}","bos_token":"<s>"}"#)],
    Expect::Exact("<s>hi s")
)]
#[case::config_without_template_uses_file(
    Call::Template,
    "unlisted/config-and-file",
    None,
    vec![
        Answer::Status(200, br#"{"bos_token":"<s>","eos_token":"<|e|>"}"#),
        Answer::Status(200, b"{{ bos_token }}{{ messages[1]['content'] }}{{ eos_token }}"),
    ],
    Expect::Exact("<s>u<|e|>")
)]
#[case::nothing_found(
    Call::Template,
    "unlisted/nothing",
    None,
    vec![Answer::Status(404, b""), Answer::Status(404, b""), Answer::Status(404, b"")],
    Expect::NoTemplate
)]
#[case::bedrock_caller_renders_builtin(Call::Selector, "meta.llama3-70b-instruct-v1:0", Some("bedrock"), vec![], Expect::Exact(LLAMA3_RENDERED))]
#[case::generic_llama3_caller_renders_builtin(Call::Selector, "meta-llama/llama-3-8b-instruct", None, vec![], Expect::Exact(LLAMA3_RENDERED))]
#[case::generic_dashed_llama3_caller_renders_builtin(Call::Selector, "meta-llama-3-8b-instruct", None, vec![], Expect::Exact(LLAMA3_RENDERED))]
#[case::final_caller_falls_back_to_default_pt(Call::Selector, "gpt2", None, vec![], Expect::Exact("s u"))]
#[case::final_caller_renders_the_original_model(
    Call::Selector,
    "Org/Model-X",
    None,
    vec![Answer::Status(200, br#"{"chat_template":"{{ messages[1]['content'] }}-ok"}"#)],
    Expect::Exact("u-ok")
)]
#[case::bedrock_caller_lets_the_error_through(Call::Uncached, "meta.llama3-70b-instruct-v1:0", Some("bedrock"), vec![], Expect::NoTemplate)]
#[case::generic_llama3_caller_falls_back(Call::Uncached, "meta-llama/llama-3-8b-instruct", None, vec![], Expect::Exact("s u"))]
#[tokio::test]
async fn hub_templates_follow_litellm(
    #[case] call: Call,
    #[case] model: &'static str,
    #[case] provider: Option<&'static str>,
    #[case] answers: Vec<Answer>,
    #[case] expected: Expect,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = if answers.is_empty() {
        Hub::Unreachable
    } else {
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        Hub::Http(hub_client(&base_url))
    };
    let cache = match call {
        Call::Uncached => BTreeMap::new(),
        Call::Selector | Call::Template => builtin_entries(),
    };
    let factory = PromptFactory {
        cache: Mutex::new(cache),
        hub,
    };
    let messages = bare_messages();
    let now = clock();
    let (output, requests) = match call {
        Call::Template => {
            let (rendered, requests) = Box::pin(fetch_case(
                &listener,
                &answers,
                factory.hf_chat_template(model, &messages, now),
            ))
            .await;
            (rendered.map(Output::Text), requests)
        }
        Call::Selector | Call::Uncached => {
            let selection = prompt_factory(&factory, model, &messages, provider, now);
            Box::pin(fetch_case(&listener, &answers, selection)).await
        }
    };
    assert_eq!(requests.len(), answers.len());
    if let Some(first) = requests.first() {
        let config_path = format!("/{model}/raw/main/tokenizer_config.json");
        assert_eq!(request_path(first), config_path);
    }
    match expected {
        Expect::Exact(text) => assert_eq!(output.unwrap(), Output::Text(text.to_owned())),
        Expect::NoTemplate => assert!(matches!(output, Err(FactoryError::NoChatTemplate))),
    }
}

async fn fetch_case<T, F>(listener: &TcpListener, answers: &[Answer], call: F) -> (T, Vec<String>)
where
    F: Future<Output = T>,
{
    match answers {
        [] => (call.await, Vec::new()),
        [one] => {
            let (result, first) = tokio::join!(call, respond(listener, one));
            (result, vec![first])
        }
        [one, two] => {
            let (result, first, second) =
                tokio::join!(call, respond(listener, one), respond(listener, two));
            (result, vec![first, second])
        }
        [one, two, three] => {
            let (result, first, second, third) = tokio::join!(
                call,
                respond(listener, one),
                respond(listener, two),
                respond(listener, three)
            );
            (result, vec![first, second, third])
        }
        _ => panic!("the case carries more answers than the fetch can use"),
    }
}
