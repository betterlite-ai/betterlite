#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::iter::repeat_n;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use super::{
    BUILTIN_ENTRIES, Hub, HubClient, HubError, MAX_FETCHED_ENTRIES, TokenizerConfig,
    builtin_entries,
};
use crate::builtin::Message;
use crate::error::FactoryError;
use crate::factory::PromptFactory;

const CONFIG_WITHOUT_TEMPLATE: &[u8] = br#"{"add_bos_token":true}"#;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Answer {
    Status(u16, &'static [u8]),
    Redirect(&'static str),
    Broken,
}

#[derive(Clone, Copy)]
enum Expect {
    Rendered(&'static str),
    // the path every redirect points at, and Some(text) to render or None for a cap hit
    Redirect {
        hop: &'static str,
        rendered: Option<&'static str>,
    },
    NoTemplate,
    NotText,
    Json,
    Transport,
    CacheRecovery,
}

fn redirect_chain(redirects: usize) -> Vec<Answer> {
    repeat_n(Answer::Redirect("/hop"), redirects).collect()
}

fn redirects_then_config(redirects: usize) -> Vec<Answer> {
    let mut answers = redirect_chain(redirects);
    answers.push(Answer::Status(200, br#"{"chat_template":"hop"}"#));
    answers
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

async fn serve(listener: &TcpListener, answers: &[&Answer]) -> Vec<String> {
    let mut requests = Vec::new();
    for answer in answers {
        requests.push(respond(listener, answer).await);
    }
    requests
}

// a scripted case that makes a different number of requests than it scripts fails here, not at the harness timeout
async fn exchange<T, F>(listener: &TcpListener, answers: &[&Answer], call: F) -> (T, Vec<String>)
where
    F: Future<Output = T>,
{
    let exchange = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(call, serve(listener, answers))
    });
    exchange
        .await
        .expect("the case made every request it scripts")
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
    Expect::Redirect { hop: "/moved/tokenizer_config.json", rendered: Some("moved") }
)]
#[case::config_twenty_redirects(
    "org/model",
    redirects_then_config(20),
    Expect::Redirect { hop: "/hop", rendered: Some("hop") }
)]
#[case::config_too_many_redirects("org/model", redirect_chain(21), Expect::Redirect { hop: "/hop", rendered: None })]
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
    if matches!(expected, Expect::Redirect { .. }) {
        redirect_case(&factory, &listener, model, &answers, expected).await;
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
        Expect::Redirect { .. } => panic!("the redirect cases have their own driver"),
    }
}

async fn redirect_case(
    factory: &PromptFactory,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
    expected: Expect,
) {
    let Expect::Redirect { hop, rendered } = expected else {
        panic!("the redirect case needs a redirect expectation");
    };
    let messages = bare_messages();
    let now = clock();
    let replies: Vec<&Answer> = answers.iter().collect();
    let (outcome, requests) = exchange(
        listener,
        &replies,
        factory.hf_chat_template(model, &messages, now),
    )
    .await;
    assert_eq!(requests.len(), answers.len());
    let config_path = format!("/{model}/raw/main/tokenizer_config.json");
    assert_eq!(
        requests.first().map(|request| request_path(request)),
        Some(config_path.as_str())
    );
    for request in requests.iter().skip(1) {
        assert_eq!(request_path(request), hop);
    }
    match rendered {
        Some(text) => assert_eq!(outcome.unwrap(), text),
        None => assert!(matches!(
            outcome,
            Err(FactoryError::Hub(HubError::Redirect { .. }))
        )),
    }
}

async fn fetch_case(
    factory: &PromptFactory,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
) -> (Result<String, FactoryError>, Vec<String>) {
    let messages = bare_messages();
    let now = clock();
    let replies: Vec<&Answer> = answers.iter().collect();
    exchange(
        listener,
        &replies,
        factory.hf_chat_template(model, &messages, now),
    )
    .await
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
    let (first, requests) = exchange(
        listener,
        &[config_reply, jinja_gone, extension_two],
        factory.hf_chat_template(model, &messages, now),
    )
    .await;
    assert!(matches!(first, Err(FactoryError::NoChatTemplate)));
    let wanted = expected_paths(model, 3);
    let requested: Vec<String> = requests
        .iter()
        .map(|request| request_path(request).to_owned())
        .collect();
    assert_eq!(requested, wanted);
    let (second, retry_requests) = exchange(
        listener,
        &[retried],
        factory.hf_chat_template(model, &messages, now),
    )
    .await;
    let retry_request = retry_requests.first().expect("the retry makes one request");
    assert_eq!(
        request_path(retry_request),
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
    let (first, _) = exchange(
        &listener,
        &[&answer],
        factory.hf_chat_template("org/model", &messages, now),
    )
    .await;
    assert_eq!(first.unwrap(), "xx");
    let (second, _) = exchange(
        &listener,
        &[&answer],
        factory.hf_chat_template("org/model", &messages, now),
    )
    .await;
    assert_eq!(second.unwrap(), "xx");
    assert!(!factory.cache.lock().unwrap().contains_key("org/model"));
}
