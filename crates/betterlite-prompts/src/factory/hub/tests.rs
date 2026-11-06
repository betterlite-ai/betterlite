#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::{
    BUILTIN_ENTRIES, Hub, HubClient, HubError, MAX_FETCHED_ENTRIES, TokenizerConfig,
    builtin_entries,
};
use crate::builtin::Message;
use crate::error::FactoryError;
use crate::factory::PromptFactory;

const CONFIG_WITHOUT_TEMPLATE: &[u8] = br#"{"add_bos_token":true}"#;

enum Answer {
    Status(u16, &'static [u8]),
    Redirect(&'static str),
    Broken,
}

#[derive(Clone, Copy)]
enum Expect {
    Rendered(&'static str),
    Redirect(&'static str),
    NoTemplate,
    NotText,
    Json,
    Transport,
    CacheRecovery,
}

fn clock() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn bare_messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#)
        .unwrap()
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

async fn respond(listener: &TcpListener, answer: &Answer) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_request(&mut stream).await;
    match answer {
        Answer::Status(status, body) => {
            let head = format!(
                "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        }
        Answer::Redirect(location) => {
            let head = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).await.unwrap();
        }
        Answer::Broken => {}
    }
    request
}

fn request_path(request: &str) -> &str {
    request.split_whitespace().nth(1).unwrap()
}

fn expected_paths(model: &str, count: usize) -> Vec<String> {
    let mut paths = vec![format!("/{model}/raw/main/tokenizer_config.json")];
    if count > 1 {
        paths.push(format!("/{model}/raw/main/chat_template.jinja"));
    }
    if count > 2 {
        paths.push(format!("/{model}/raw/main/chat_template.jinja2"));
    }
    paths
}

#[rstest]
#[case::config_success(
    "org/model",
    vec![Answer::Status(200, br#"{"chat_template":"{{ bos_token }}{{ eos_token }}","bos_token":"<s>","eos_token":{"content":"</s>"}}"#)],
    Expect::Rendered("<s></s>")
)]
#[case::config_not_found(
    "org/model",
    vec![Answer::Status(404, b""), Answer::Status(404, b""), Answer::Status(404, b"")],
    Expect::NoTemplate
)]
#[case::config_bad_json(
    "org/model",
    vec![Answer::Status(200, b"not json")],
    Expect::Json
)]
#[case::config_non_text_template(
    "org/model",
    vec![Answer::Status(200, br#"{"chat_template":["first","second"]}"#)],
    Expect::NotText
)]
#[case::config_array_body(
    "org/model",
    vec![
        Answer::Status(200, br#"["abc","<s>"]"#),
        Answer::Status(200, b"[{{ bos_token }}]")
    ],
    Expect::Rendered("[]")
)]
#[case::config_non_text_token(
    "org/model",
    vec![Answer::Status(200, br#"{"chat_template":"{{ bos_token }}x","bos_token":5}"#)],
    Expect::Rendered("x")
)]
#[case::config_transport_error(
    "org/model",
    vec![Answer::Broken],
    Expect::Transport
)]
#[case::template_jinja(
    "org/model",
    vec![Answer::Status(200, CONFIG_WITHOUT_TEMPLATE), Answer::Status(200, b"t1")],
    Expect::Rendered("t1")
)]
#[case::template_jinja2_after_missing_jinja(
    "org/model",
    vec![
        Answer::Status(200, CONFIG_WITHOUT_TEMPLATE),
        Answer::Status(404, b""),
        Answer::Status(200, b"t2")
    ],
    Expect::Rendered("t2")
)]
#[case::template_bad_utf8_moves_on(
    "org/model",
    vec![
        Answer::Status(200, CONFIG_WITHOUT_TEMPLATE),
        Answer::Status(200, b"\xff"),
        Answer::Status(200, b"t3")
    ],
    Expect::Rendered("t3")
)]
#[case::template_transport_error_moves_on(
    "org/model",
    vec![
        Answer::Status(200, CONFIG_WITHOUT_TEMPLATE),
        Answer::Broken,
        Answer::Status(200, b"t4")
    ],
    Expect::Rendered("t4")
)]
#[case::template_nothing(
    "org/model",
    vec![
        Answer::Status(200, CONFIG_WITHOUT_TEMPLATE),
        Answer::Status(404, b""),
        Answer::Status(404, b"")
    ],
    Expect::NoTemplate
)]
#[case::cache_stores_a_failure(
    "org/model",
    vec![
        Answer::Status(404, b""),
        Answer::Status(404, b""),
        Answer::Status(404, b""),
        Answer::Status(200, b"recovered")
    ],
    Expect::CacheRecovery
)]
#[case::config_redirect(
    "org/model",
    vec![
        Answer::Redirect("/moved/tokenizer_config.json"),
        Answer::Status(200, br#"{"chat_template":"moved","bos_token":"<s>"}"#)
    ],
    Expect::Redirect("moved")
)]
#[tokio::test]
async fn fetch_follows_litellm(
    #[case] model: &'static str,
    #[case] answers: Vec<Answer>,
    #[case] expected: Expect,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let factory = PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Http(hub_client(&base_url)),
    };
    if matches!(expected, Expect::CacheRecovery) {
        cached_failure_case(&factory, &listener, model, &answers).await;
        return;
    }
    if let Expect::Redirect(text) = expected {
        redirect_case(&factory, &listener, model, &answers, text).await;
        return;
    }
    let (rendered, requests) = fetch_case(&factory, &listener, model, &answers).await;
    let wanted = expected_paths(model, answers.len());
    let requested: Vec<String> = requests
        .iter()
        .map(|request| request_path(request).to_owned())
        .collect();
    assert_eq!(requested, wanted);
    match expected {
        Expect::Rendered(text) => assert_eq!(rendered.unwrap(), text),
        Expect::NoTemplate => assert!(matches!(rendered, Err(FactoryError::NoChatTemplate))),
        Expect::NotText => assert!(matches!(rendered, Err(FactoryError::TemplateNotText))),
        Expect::Json => assert!(matches!(
            rendered,
            Err(FactoryError::Hub(HubError::Json { .. }))
        )),
        Expect::Transport => assert!(matches!(
            rendered,
            Err(FactoryError::Hub(HubError::Transport { .. }))
        )),
        Expect::CacheRecovery => panic!("the cached case has its own driver"),
        Expect::Redirect(_) => panic!("the redirect case has its own driver"),
    }
}

async fn redirect_case(
    factory: &PromptFactory,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
    expected: &str,
) {
    let [redirected, destination] = answers else {
        panic!("the redirect case needs two answers");
    };
    let messages = bare_messages();
    let now = clock();
    let (rendered, config, moved) = tokio::join!(
        factory.hf_chat_template(model, &messages, now),
        respond(listener, redirected),
        respond(listener, destination),
    );
    assert_eq!(
        request_path(&config),
        format!("/{model}/raw/main/tokenizer_config.json")
    );
    assert_eq!(request_path(&moved), "/moved/tokenizer_config.json");
    assert_eq!(rendered.unwrap(), expected);
}

async fn fetch_case(
    factory: &PromptFactory,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
) -> (Result<String, FactoryError>, Vec<String>) {
    let messages = bare_messages();
    let now = clock();
    match answers {
        [one] => {
            let (rendered, first) = tokio::join!(
                factory.hf_chat_template(model, &messages, now),
                respond(listener, one)
            );
            (rendered, vec![first])
        }
        [one, two] => {
            let (rendered, first, second) = tokio::join!(
                factory.hf_chat_template(model, &messages, now),
                respond(listener, one),
                respond(listener, two)
            );
            (rendered, vec![first, second])
        }
        [one, two, three] => {
            let (rendered, first, second, third) = tokio::join!(
                factory.hf_chat_template(model, &messages, now),
                respond(listener, one),
                respond(listener, two),
                respond(listener, three)
            );
            (rendered, vec![first, second, third])
        }
        _ => panic!("the case carries more answers than the fetch can use"),
    }
}

async fn cached_failure_case(
    factory: &PromptFactory,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
) {
    let [config_reply, jinja_gone, extension_two, retried] = answers else {
        panic!("the case needs four answers");
    };
    let messages = bare_messages();
    let now = clock();
    let (first, config, jinja, jinja2) = tokio::join!(
        factory.hf_chat_template(model, &messages, now),
        respond(listener, config_reply),
        respond(listener, jinja_gone),
        respond(listener, extension_two),
    );
    assert!(matches!(first, Err(FactoryError::NoChatTemplate)));
    assert_eq!(
        request_path(&config),
        format!("/{model}/raw/main/tokenizer_config.json")
    );
    assert_eq!(
        request_path(&jinja),
        format!("/{model}/raw/main/chat_template.jinja")
    );
    assert_eq!(
        request_path(&jinja2),
        format!("/{model}/raw/main/chat_template.jinja2")
    );
    let (second, retry_request) = tokio::join!(
        factory.hf_chat_template(model, &messages, now),
        respond(listener, retried),
    );
    assert_eq!(
        request_path(&retry_request),
        format!("/{model}/raw/main/chat_template.jinja")
    );
    assert_eq!(second.unwrap(), "recovered");
}

#[tokio::test]
async fn template_cache_caps_fetched_entries() {
    let mut entries = builtin_entries();
    assert_eq!(entries.len(), BUILTIN_ENTRIES);
    entries.extend(
        (0..MAX_FETCHED_ENTRIES).map(|index| (format!("filler{index}"), TokenizerConfig::Failure)),
    );
    // the deviation keeps 1024 fetched entries beside the three built-ins
    assert_eq!(entries.len(), 1027);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let factory = PromptFactory {
        cache: Mutex::new(entries),
        hub: Hub::Http(hub_client(&base_url)),
    };
    let messages = bare_messages();
    let now = clock();
    let answer = Answer::Status(200, br#"{"chat_template":"xx"}"#);
    let (first, _) = tokio::join!(
        factory.hf_chat_template("org/model", &messages, now),
        respond(&listener, &answer),
    );
    assert_eq!(first.unwrap(), "xx");
    let (second, _) = tokio::join!(
        factory.hf_chat_template("org/model", &messages, now),
        respond(&listener, &answer),
    );
    assert_eq!(second.unwrap(), "xx");
    assert!(!factory.cache.lock().unwrap().contains_key("org/model"));
}
