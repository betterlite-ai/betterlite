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
    BUILTIN_ENTRIES, ChatTemplate, ChatTemplateFile, Hub, HubClient, HubError, MAX_FETCHED_ENTRIES,
    TokenizerConfig, builtin_entries,
};
use crate::builtin::Message;
use crate::error::FactoryError;
use crate::factory::PromptFactory;
use crate::jinja::extract_token_value;

enum Answer {
    Status(u16, &'static [u8]),
    Broken,
}

#[derive(Clone, Copy)]
enum Fetch {
    Config,
    Template,
    CachedFailure,
}

#[derive(Clone, Copy)]
enum Expect {
    Config {
        template: &'static str,
        bos: &'static str,
        eos: &'static str,
    },
    Template(&'static str),
    Json,
    Transport,
    Failure,
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
    if let Answer::Status(status, body) = answer {
        let head = format!(
            "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
    }
    request
}

fn request_path(request: &str) -> &str {
    request.split_whitespace().nth(1).unwrap()
}

#[rstest]
#[case::config_success(
    Fetch::Config,
    "org/model",
    vec![Answer::Status(200, br#"{"tokenizer":{"chat_template":"{{ bos_token }}t","bos_token":"<s>","eos_token":{"content":"</s>"}}}"#)],
    Expect::Config { template: "{{ bos_token }}t", bos: "<s>", eos: "</s>" }
)]
#[case::config_not_found(
    Fetch::Config,
    "org/model",
    vec![Answer::Status(404, b"")],
    Expect::Failure
)]
#[case::config_bad_json(
    Fetch::Config,
    "org/model",
    vec![Answer::Status(200, b"not json")],
    Expect::Json
)]
#[case::config_transport_error(
    Fetch::Config,
    "org/model",
    vec![Answer::Broken],
    Expect::Transport
)]
#[case::template_jinja(
    Fetch::Template,
    "org/model",
    vec![Answer::Status(200, b"t1")],
    Expect::Template("t1")
)]
#[case::template_jinja2_after_missing_jinja(
    Fetch::Template,
    "org/model",
    vec![Answer::Status(404, b""), Answer::Status(200, b"t2")],
    Expect::Template("t2")
)]
#[case::template_bad_utf8_moves_on(
    Fetch::Template,
    "org/model",
    vec![Answer::Status(200, b"\xff"), Answer::Status(200, b"t3")],
    Expect::Template("t3")
)]
#[case::template_transport_error_moves_on(
    Fetch::Template,
    "org/model",
    vec![Answer::Broken, Answer::Status(200, b"t4")],
    Expect::Template("t4")
)]
#[case::template_nothing(
    Fetch::Template,
    "org/model",
    vec![Answer::Status(404, b""), Answer::Status(404, b"")],
    Expect::Failure
)]
#[case::cache_stores_a_failure(
    Fetch::CachedFailure,
    "org/model",
    vec![
        Answer::Status(404, b""),
        Answer::Status(404, b""),
        Answer::Status(404, b""),
        Answer::Status(200, b"recovered")
    ],
    Expect::CacheRecovery
)]
#[tokio::test]
async fn fetch_follows_litellm(
    #[case] fetch: Fetch,
    #[case] model: &'static str,
    #[case] answers: Vec<Answer>,
    #[case] expected: Expect,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    match fetch {
        Fetch::Config => config_case(&base_url, &listener, model, &answers[0], expected).await,
        Fetch::Template => template_case(&base_url, &listener, model, &answers, expected).await,
        Fetch::CachedFailure => cached_failure_case(&base_url, &listener, model, &answers).await,
    }
}

async fn config_case(
    base_url: &str,
    listener: &TcpListener,
    model: &str,
    answer: &Answer,
    expected: Expect,
) {
    let client = hub_client(base_url);
    let (result, request) = tokio::join!(client.tokenizer_config(model), respond(listener, answer));
    let expected_path = format!("/{model}/raw/main/tokenizer_config.json");
    assert_eq!(request_path(&request), expected_path.as_str());
    match expected {
        Expect::Config { template, bos, eos } => {
            let TokenizerConfig::Success(document) = result.unwrap() else {
                panic!("expected a tokenizer config");
            };
            let section = document.tokenizer.unwrap();
            assert_eq!(
                section.chat_template,
                ChatTemplate::Text(template.to_owned())
            );
            assert_eq!(extract_token_value(section.bos_token), bos);
            assert_eq!(extract_token_value(section.eos_token), eos);
        }
        Expect::Json => assert!(matches!(result, Err(HubError::Json { .. }))),
        Expect::Transport => assert!(matches!(result, Err(HubError::Transport { .. }))),
        Expect::Failure => assert!(matches!(result, Ok(TokenizerConfig::Failure))),
        Expect::Template(_) | Expect::CacheRecovery => panic!("the case matches no config outcome"),
    }
}

async fn template_case(
    base_url: &str,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
    expected: Expect,
) {
    let client = hub_client(base_url);
    let (result, requests) = match answers {
        [one] => {
            let (result, first) =
                tokio::join!(client.chat_template_file(model), respond(listener, one));
            (result, vec![first])
        }
        [one, two] => {
            let (result, first, second) = tokio::join!(
                client.chat_template_file(model),
                respond(listener, one),
                respond(listener, two)
            );
            (result, vec![first, second])
        }
        _ => panic!("the case needs one or two answers"),
    };
    let requested: Vec<String> = requests
        .iter()
        .map(|request| request_path(request).to_owned())
        .collect();
    let mut wanted_paths = vec![format!("/{model}/raw/main/chat_template.jinja")];
    if answers.len() == 2 {
        wanted_paths.push(format!("/{model}/raw/main/chat_template.jinja2"));
    }
    assert_eq!(requested, wanted_paths);
    match expected {
        Expect::Template(text) => assert_eq!(result, ChatTemplateFile::Success(text.to_owned())),
        Expect::Failure => assert_eq!(result, ChatTemplateFile::Failure),
        Expect::Config { .. } | Expect::Json | Expect::Transport | Expect::CacheRecovery => {
            panic!("the case matches no template outcome");
        }
    }
}

async fn cached_failure_case(
    base_url: &str,
    listener: &TcpListener,
    model: &str,
    answers: &[Answer],
) {
    let factory = PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Http(hub_client(base_url)),
    };
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
    entries.extend(
        (0..MAX_FETCHED_ENTRIES).map(|index| (format!("filler{index}"), TokenizerConfig::Failure)),
    );
    assert_eq!(entries.len(), BUILTIN_ENTRIES + MAX_FETCHED_ENTRIES);
    let cache = Mutex::new(entries);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let factory = PromptFactory {
        cache,
        hub: Hub::Http(hub_client(&base_url)),
    };
    let messages = bare_messages();
    let now = clock();
    let answer = Answer::Status(200, br#"{"tokenizer":{"chat_template":"xx"}}"#);
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
