#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};
use betterlite_tokenizer::count::settings::Settings as TokenizerSettings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use tokio::time::Instant;

use super::estimate::{TokenEstimateRequest, estimate_precise_input_tokens};
use super::support::{FixedResponses, no_translation, request};

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

fn hub_downloader(client: &Arc<ProxiedClient>) -> Arc<HubDownloader> {
    let hub = HubSettings {
        endpoint: "http://127.0.0.1:1".to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::from("unused"),
        hub_cache: PathBuf::from("unused"),
        token_path: PathBuf::from("unused-token"),
        stored_tokens_path: PathBuf::from("unused-stored-tokens"),
        transfer: TransferSetting::Standard,
    };
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let resolver = Arc::new(HubTokenResolver::new(
        hub.clone(),
        OidcSettings::default(),
        Arc::clone(client),
        reader,
        clock,
    ));
    Arc::new(HubDownloader::new(
        Arc::new(hub),
        resolver,
        Arc::new(Instant::now),
    ))
}

fn token_counter() -> Counter {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the default proxy policy");
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).expect("an outbound client"));
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        EncodeLimits::default(),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let catalog = BundledCatalog::default().get();
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        hub_downloader(&client),
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(
        selector,
        fetcher,
        service,
        catalog,
        TokenizerSettings::default(),
    )
}

async fn counted(call_type: &str, body: &str) -> i64 {
    let data = request(body);
    estimate_precise_input_tokens(
        &no_translation(),
        &FixedResponses {
            messages: Vec::new(),
        },
        &token_counter(),
        &TokenEstimateRequest {
            data: &data,
            call_type: Some(call_type),
            model: Some("gpt-3.5-turbo"),
            min_configured_tpm_limit: None,
            configured_output_tokens: None,
        },
    )
    .await
}

const SENTENCE: &str = "the quick brown fox jumps over the lazy dog";

async fn direct_count(text: Option<&TextInput>, messages: Option<&[Message]>) -> i64 {
    let count = token_counter()
        .count(&TokenRequest {
            model: "gpt-3.5-turbo",
            custom_tokenizer: None,
            text,
            messages,
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        })
        .await
        .expect("the tokenizer counts the input");
    i64::try_from(count).expect("a count that fits an i64")
}

#[tokio::test]
async fn the_tokenizer_counts_a_text_prompt_by_model() {
    let body = format!(r#"{{"prompt":"{SENTENCE}"}}"#);
    let expected = direct_count(Some(&TextInput::One(SENTENCE.to_owned())), None).await;
    assert_eq!(counted("text_completion", &body).await, expected);
    assert_ne!(expected, 10, "the char estimate of the sentence is 10");
}

#[tokio::test]
async fn the_tokenizer_joins_a_list_of_prompts() {
    let list = format!(r#"{{"prompt":["{SENTENCE}","{SENTENCE}"]}}"#);
    let joined = format!(r#"{{"prompt":"{SENTENCE}{SENTENCE}"}}"#);
    let single = counted("text_completion", &format!(r#"{{"prompt":"{SENTENCE}"}}"#)).await;
    let expected = counted("text_completion", &joined).await;
    assert!(expected > single);
    assert_eq!(counted("text_completion", &list).await, expected);
}

#[tokio::test]
async fn a_prompt_list_with_a_non_text_item_falls_back_to_the_char_estimate() {
    let body = format!(r#"{{"prompt":["{SENTENCE}",5]}}"#);
    assert_eq!(counted("text_completion", &body).await, 11);
}

#[tokio::test]
async fn the_tokenizer_counts_chat_messages_with_their_overhead() {
    let body = r#"{"messages":[{"role":"user","content":"hello world"}]}"#;
    let message: Message =
        serde_json::from_str(r#"{"role":"user","content":"hello world"}"#).expect("a chat message");
    let expected = direct_count(None, Some(&[message])).await;
    assert_eq!(counted("completion", body).await, expected);
    assert!(expected > 2, "the char estimate of the message is 2");
}
