use std::sync::Arc;

use betterlite_config::sections::huggingface_hub::Settings as HubSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use rstest::rstest;
use secrecy::SecretString;

use crate::engine::Engine;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::hub::{HubDownloader, HubError};
use crate::engine::limits::CountingService;
use crate::engine::selection::{CustomTokenizer, Kind, SelectionError, Selector, TokenizerType};
use crate::engine::tests::support::{
    InlineCounter, Reply, Request, Server, client, hub_settings, resolver, system_clock,
};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const CATALOG: &str = r#"{
  "command-r": {"litellm_provider": "cohere"},
  "claude-2.1": {"litellm_provider": "anthropic"},
  "claude-3-opus-20240229": {"litellm_provider": "anthropic"},
  "meta-llama/llama-2-70b-chat": {"litellm_provider": "replicate"},
  "meta-llama/Meta-Llama-3-70B": {"litellm_provider": "bedrock"}
}"#;
const TOKENIZER_JSON: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2},"unk_token":"[UNK]"}}"#;

fn tokenizer_reply(request: &Request) -> Reply {
    if request.method == "HEAD" {
        return Reply::Response {
            status: 200,
            headers: vec![
                ("ETag", "\"etag-value\"".to_owned()),
                ("X-Repo-Commit", COMMIT.to_owned()),
                ("Content-Length", TOKENIZER_JSON.len().to_string()),
            ],
            body: Vec::new(),
        };
    }
    Reply::Response {
        status: 200,
        headers: Vec::new(),
        body: TOKENIZER_JSON.as_bytes().to_vec(),
    }
}

fn selector(settings: &HubSettings, disable: bool) -> Selector {
    let hub = Arc::new(HubDownloader::new(
        Arc::new(settings.clone()),
        resolver(settings),
        system_clock(),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(client(), None));
    let catalog = Arc::new(ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap());
    let counter = Arc::new(CountingService::new(4, Arc::new(InlineCounter)));
    Selector::new(fetcher, hub, catalog, counter, disable)
}

#[rstest]
#[case::cohere("command-r", TokenizerType::HuggingFace, "Xenova/c4ai-command-r-v01-tokenizer", &[1, 2])]
#[case::anthropic("claude-2.1", TokenizerType::HuggingFace, "", &[9381, 2253])]
#[case::llama2("meta-llama/llama-2-70b-chat", TokenizerType::HuggingFace, "hf-internal-testing/llama-tokenizer", &[1, 2])]
#[case::llama3("meta-llama/Meta-Llama-3-70B", TokenizerType::HuggingFace, "Xenova/llama-3-tokenizer", &[1, 2])]
#[case::replicate_llama3_is_llama2("replicate/some-llama-3-model", TokenizerType::HuggingFace, "hf-internal-testing/llama-tokenizer", &[1, 2])]
#[case::claude3_is_cl100k("claude-3-opus-20240229", TokenizerType::OpenAi, "", &[15339, 1917])]
#[case::unknown_is_cl100k("gpt-4o", TokenizerType::OpenAi, "", &[15339, 1917])]
#[tokio::test]
async fn every_kind_matches_litellm(
    #[case] model: &str,
    #[case] expected_type: TokenizerType,
    #[case] expected_repo: &str,
    #[case] expected_ids: &[u32],
) {
    let server = Server::start(tokenizer_reply).await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let selected = selector(&settings, false)
        .select(model, None)
        .await
        .unwrap();
    assert_eq!(selected.1, expected_type);
    assert_eq!(selected.0.encode("hello world").unwrap(), expected_ids);
    match expected_repo {
        "" => assert_eq!(server.request_count(), 0),
        repo => assert!(
            server
                .last_path()
                .is_some_and(|path: String| path.contains(repo))
        ),
    }
}

#[tokio::test]
async fn the_disable_switch_returns_cl100k() {
    let server = Server::start(tokenizer_reply).await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let selected = selector(&settings, true)
        .select("command-r", None)
        .await
        .unwrap();
    assert_eq!(selected.1, TokenizerType::OpenAi);
    assert_eq!(server.request_count(), 0);
}

#[tokio::test]
async fn each_kind_is_downloaded_once() {
    let server = Server::start(tokenizer_reply).await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let selector = selector(&settings, false);
    let first = selector
        .select("meta-llama/Meta-Llama-3-70B", None)
        .await
        .unwrap();
    assert_eq!(first.1, TokenizerType::HuggingFace);
    let second = selector
        .select("meta-llama/Meta-Llama-3-70B", None)
        .await
        .unwrap();
    assert_eq!(second.1, TokenizerType::HuggingFace);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_failed_download_falls_back_to_cl100k() {
    let server = Server::start(|_request| Reply::Response {
        status: 404,
        headers: Vec::new(),
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let selected = selector(&settings, false)
        .select("meta-llama/Meta-Llama-3-70B", None)
        .await
        .unwrap();
    assert_eq!(selected.1, TokenizerType::OpenAi);
    assert_eq!(selected.0.encode("hello world").unwrap(), [15339, 1917]);
}

#[tokio::test]
async fn a_custom_tokenizer_failure_reaches_the_caller() {
    let server = Server::start(|_request| Reply::Response {
        status: 404,
        headers: Vec::new(),
        body: Vec::new(),
    })
    .await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let custom = CustomTokenizer {
        identifier: "owner/private".to_owned(),
        revision: "main".to_owned(),
        auth_token: Some(SecretString::from("secret")),
    };
    assert!(matches!(
        selector(&settings, false)
            .select("command-r", Some(&custom))
            .await,
        Err(SelectionError::Hub(HubError::RepoNotFound(..)))
    ));
}

#[tokio::test]
async fn a_custom_tokenizer_is_downloaded_once_with_its_token() {
    let server = Server::start(tokenizer_reply).await;
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings(&server.endpoint, directory.path());
    let custom = CustomTokenizer {
        identifier: "owner/private".to_owned(),
        revision: "rev-9".to_owned(),
        auth_token: Some(SecretString::from("secret")),
    };
    let selector = selector(&settings, false);
    let first = selector.select("command-r", Some(&custom)).await.unwrap();
    assert_eq!(first.0.encode("hello world").unwrap(), [1, 2]);
    let second = selector.select("command-r", Some(&custom)).await.unwrap();
    assert_eq!(second.0.encode("hello world").unwrap(), [1, 2]);
    assert_eq!(server.request_count(), 2);
    let requests = server.requests.lock().unwrap();
    let request = requests.first().unwrap();
    let authorization = request.header("authorization").map(str::to_owned);
    let path = request.path.clone();
    drop(requests);
    assert_eq!(authorization.as_deref(), Some("Bearer secret"));
    assert!(path.contains("/owner/private/resolve/rev-9/tokenizer.json"));
}

#[tokio::test]
async fn default_encoding_is_the_bundled_cl100k() {
    let directory = tempfile::tempdir().unwrap();
    let settings = hub_settings("http://127.0.0.1:1", directory.path());
    let selected = selector(&settings, false)
        .load_kind(Kind::OpenAi)
        .await
        .unwrap();
    assert!(matches!(selected, Engine::Tiktoken(_)));
    assert_eq!(selected.encode("hello world").unwrap(), [15339, 1917]);
}
