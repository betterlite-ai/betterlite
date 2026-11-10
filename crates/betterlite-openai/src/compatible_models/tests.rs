#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::error::ModelInfoError;
use super::{
    MODEL_INFO_DISCOVERY_PROVIDERS, MODEL_INFO_REFRESH_CONCURRENCY, MODEL_INFO_REFRESH_SECONDS,
    ModelCard, ModelList, ModelListCache, ModelQuery, TokenLimits, cache_key,
    decode_compatible_model_list, discover_model_limits,
};
use crate::openai_base_config::Headers;

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const TWO_MIB: usize = 2 * 1024 * 1024;
const ONE_MODEL: &str = r#"{"data":[{"id":"m","max_model_len":4096,"max_output_tokens":1024}]}"#;

#[derive(Clone, Default)]
struct DebugMessages(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for DebugMessages {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() != &tracing::Level::DEBUG {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        if let Some(message) = field.0 {
            self.0.lock().unwrap().push(message);
        }
    }
}

#[derive(Default)]
struct MessageField(Option<String>);

impl Visit for MessageField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

#[derive(Default)]
struct Recorder {
    held: Mutex<BTreeMap<String, ModelList>>,
    stored: Mutex<Vec<(String, Duration)>>,
}

impl ModelListCache for Recorder {
    fn get(&self, key: &str) -> Option<ModelList> {
        self.held.lock().unwrap().get(key).cloned()
    }

    fn set(&self, key: &str, list: ModelList, ttl: Duration) {
        self.held.lock().unwrap().insert(key.to_owned(), list);
        self.stored.lock().unwrap().push((key.to_owned(), ttl));
    }
}

fn parsed_card(text: &str) -> ModelCard {
    serde_json::from_str(text).unwrap()
}

fn limits(max_tokens: Option<u64>, input: Option<u64>, output: Option<u64>) -> TokenLimits {
    TokenLimits {
        max_tokens,
        max_input_tokens: input,
        max_output_tokens: output,
    }
}

fn auth(value: &str) -> Headers {
    Headers::from([("Authorization".to_owned(), value.to_owned())])
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(request).unwrap();
        }
    }
}

async fn serve(listener: &TcpListener, status: u16, extra_head: &str, body: &[u8]) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_request(&mut stream).await;
    let head = format!(
        "HTTP/1.1 {status} X\r\n{extra_head}content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).await.is_ok() {
        stream.write_all(body).await.unwrap_or_default();
    }
    request
}

async fn discover(
    cache: &Recorder,
    api_base: &str,
    headers: &Headers,
) -> Result<TokenLimits, ModelInfoError> {
    let query = ModelQuery {
        model: "m",
        api_base,
        headers,
    };
    discover_model_limits(&client(), cache, &query).await
}

async fn discovered(
    status: u16,
    extra_head: &str,
    body: &[u8],
) -> (TokenLimits, Vec<String>, Vec<(String, Duration)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cache = Recorder::default();
    let headers = auth("Bearer a");
    let (found, request) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(
            discover(&cache, &base, &headers),
            serve(&listener, status, extra_head, body)
        )
    })
    .await
    .unwrap();
    let stored = cache.stored.lock().unwrap().clone();
    let requests = vec![request];
    (found.unwrap(), requests, stored)
}

#[test]
fn the_refresh_constants_and_provider_set_are_exported() {
    assert_eq!(MODEL_INFO_REFRESH_SECONDS, 300);
    assert_eq!(MODEL_INFO_REFRESH_CONCURRENCY, 8);
    let mut providers = MODEL_INFO_DISCOVERY_PROVIDERS.to_vec();
    providers.sort_unstable();
    assert_eq!(
        providers,
        [
            "hosted_vllm",
            "openai",
            "openai_like",
            "text-completion-openai"
        ]
    );
}

#[rstest]
#[case::context_and_output(
    r#"{"id":"m","max_model_len":4096,"max_output_tokens":1024}"#,
    limits(Some(4096), Some(4096), Some(1024))
)]
#[case::limits_clamp_to_the_context(
    r#"{"id":"m","max_model_len":2048,"max_input_tokens":9999,"max_output_tokens":5000}"#,
    limits(Some(2048), Some(2048), Some(2048))
)]
#[case::limit_equal_to_the_context(
    r#"{"id":"m","max_model_len":2048,"max_input_tokens":2048}"#,
    limits(Some(2048), Some(2048), Some(2048))
)]
#[case::model_len_beats_context_length(
    r#"{"id":"m","max_model_len":4096,"context_length":512}"#,
    limits(Some(4096), Some(4096), Some(4096))
)]
#[case::context_length_alone(
    r#"{"id":"m","context_length":512}"#,
    limits(Some(512), Some(512), Some(512))
)]
#[case::explicit_limits_without_a_context(
    r#"{"id":"m","max_input_tokens":7,"max_output_tokens":9}"#,
    limits(None, Some(7), Some(9))
)]
#[case::input_only(r#"{"id":"m","max_input_tokens":300}"#, limits(None, Some(300), None))]
#[case::bool_is_not_a_limit(
    r#"{"id":"m","max_model_len":true,"max_input_tokens":5}"#,
    limits(None, Some(5), None)
)]
#[case::zero_is_not_a_limit(
    r#"{"id":"m","max_model_len":0,"context_length":8}"#,
    limits(Some(8), Some(8), Some(8))
)]
#[case::negative_is_not_a_limit(r#"{"id":"m","max_model_len":-4}"#, limits(None, None, None))]
#[case::float_is_not_a_limit(r#"{"id":"m","max_model_len":4096.0}"#, limits(None, None, None))]
#[case::text_is_not_a_limit(r#"{"id":"m","max_model_len":"4096"}"#, limits(None, None, None))]
#[case::null_is_not_a_limit(
    r#"{"id":"m","max_model_len":null,"context_length":6}"#,
    limits(Some(6), Some(6), Some(6))
)]
#[case::no_limits(r#"{"id":"m"}"#, limits(None, None, None))]
fn token_limits_take_positive_integers_and_clamp_to_the_context(
    #[case] card: &str,
    #[case] expected: TokenLimits,
) {
    assert_eq!(parsed_card(card).token_limits(), expected);
}

#[rstest]
#[case::list(ONE_MODEL, true)]
#[case::no_data_member("{}", true)]
#[case::empty_data(r#"{"data":[]}"#, true)]
#[case::null_data(r#"{"data":null}"#, false)]
#[case::card_without_an_id(r#"{"data":[{"max_model_len":5}]}"#, false)]
#[case::not_an_object("[]", false)]
#[case::card_is_a_sequence(r#"{"data":[["m",null,null,null,null]]}"#, false)]
#[case::truncated(r#"{"data":"#, false)]
fn a_model_list_decodes_only_when_every_card_does(#[case] body: &str, #[case] decoded: bool) {
    assert_eq!(
        decode_compatible_model_list(body.as_bytes()).is_ok(),
        decoded
    );
}

#[test]
fn the_cache_key_hashes_python_json_of_the_url_and_sorted_headers() {
    let headers = Headers::from([
        ("X-A".to_owned(), "1".to_owned()),
        ("Authorization".to_owned(), "Bearer a".to_owned()),
    ]);
    assert_eq!(
        cache_key("https://h/v1/models", &headers),
        "upstream_model_info:cf93a6169fe6fc67a4fc7e87e4419fdd87ced4d2fd47f4e561b7e7f937a4ad6e"
    );
    let other = Headers::from([
        ("X-A".to_owned(), "1".to_owned()),
        ("Authorization".to_owned(), "Bearer b".to_owned()),
    ]);
    assert_eq!(
        cache_key("https://h/v1/models", &other),
        "upstream_model_info:228c313bd82fe8cc886bc0e5a97138189a5aa7ee1e8dbfd279f902bf25895a40"
    );
    assert_ne!(
        cache_key("https://h/v1/models", &headers),
        cache_key("https://h/v1/models", &other)
    );
    assert_ne!(
        cache_key("https://h/v1/models", &headers),
        cache_key("https://other/v1/models", &headers)
    );
}

#[tokio::test]
async fn a_good_list_is_fetched_from_v1_models_and_kept_for_300_seconds() {
    let (found, requests, stored) = discovered(200, "", ONE_MODEL.as_bytes()).await;
    assert_eq!(found, limits(Some(4096), Some(4096), Some(1024)));
    assert!(requests[0].starts_with("GET /v1/models HTTP/1.1"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer a")
    );
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].1, Duration::from_secs(300));
    assert!(stored[0].0.starts_with("upstream_model_info:"));
}

#[rstest]
#[case::server_error(500, ONE_MODEL)]
#[case::not_found(404, ONE_MODEL)]
#[case::not_json(200, "nope")]
#[case::truncated(200, r#"{"data":"#)]
#[case::card_without_an_id(200, r#"{"data":[{"max_model_len":5}]}"#)]
#[tokio::test]
async fn a_failed_fetch_is_an_empty_list_kept_for_60_seconds(
    #[case] status: u16,
    #[case] body: &str,
) {
    let (found, _, stored) = discovered(status, "", body.as_bytes()).await;
    assert_eq!(found, TokenLimits::default());
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].1, Duration::from_secs(60));
}

#[tokio::test]
async fn a_failed_fetch_logs_a_debug_diagnostic() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(DebugMessages(Arc::clone(&messages)));
    let _guard = set_default(subscriber);
    let (found, _, _) = discovered(500, "", b"").await;
    assert_eq!(found, TokenLimits::default());
    let seen = messages.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|message| message.contains("Could not discover upstream model token limits"))
    );
}

async fn silent_after_request(listener: &TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    drop(read_request(&mut stream).await);
    sleep(Duration::from_secs(6)).await;
}

#[tokio::test]
async fn a_slow_models_endpoint_gives_up_after_five_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cache = Recorder::default();
    let headers = Headers::new();
    let (found, ()) = timeout(Duration::from_secs(8), async {
        tokio::join!(
            discover(&cache, &base, &headers),
            silent_after_request(&listener)
        )
    })
    .await
    .expect("the five-second deadline ends the fetch");
    assert_eq!(found.unwrap(), TokenLimits::default());
    assert_eq!(cache.stored.lock().unwrap()[0].1, Duration::from_secs(60));
}

#[tokio::test]
async fn the_first_card_with_the_model_id_wins() {
    let body = r#"{"data":[{"id":"x","max_model_len":1},{"id":"m","max_model_len":10},{"id":"m","max_model_len":20}]}"#;
    let (found, _, _) = discovered(200, "", body.as_bytes()).await;
    assert_eq!(found, limits(Some(10), Some(10), Some(10)));
}

#[tokio::test]
async fn a_list_without_the_model_gives_no_limits_but_is_still_kept_for_300_seconds() {
    let (found, _, stored) =
        discovered(200, "", br#"{"data":[{"id":"x","max_model_len":1}]}"#).await;
    assert_eq!(found, TokenLimits::default());
    assert_eq!(stored[0].1, Duration::from_secs(300));
}

#[rstest]
#[case::exactly_the_cap(0, Some(10))]
#[case::one_byte_over(1, None)]
#[tokio::test]
async fn the_response_is_capped_at_2_mib(#[case] excess: usize, #[case] max_tokens: Option<u64>) {
    let head = r#"{"data":[{"id":"m","max_model_len":10}],"pad":""#;
    let padding = TWO_MIB
        .saturating_add(excess)
        .saturating_sub(head.len())
        .saturating_sub(2);
    let body = format!("{head}{}\"}}", "x".repeat(padding));
    let (found, _, stored) = discovered(200, "", body.as_bytes()).await;
    assert_eq!(found.max_tokens, max_tokens);
    let ttl = if max_tokens.is_some() { 300 } else { 60 };
    assert_eq!(stored[0].1, Duration::from_secs(ttl));
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cache = Recorder::default();
    let headers = auth("Bearer a");
    let served = async {
        let first = serve(&listener, 307, "location: /v1/other\r\n", b"").await;
        let second = timeout(
            Duration::from_millis(300),
            serve(&listener, 200, "", ONE_MODEL.as_bytes()),
        )
        .await;
        (first, second.is_err())
    };
    let (found, (first, no_second_request)) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(discover(&cache, &base, &headers), served)
    })
    .await
    .unwrap();
    assert_eq!(found.unwrap(), TokenLimits::default());
    assert!(first.starts_with("GET /v1/models HTTP/1.1"));
    assert!(no_second_request);
    assert_eq!(cache.stored.lock().unwrap()[0].1, Duration::from_secs(60));
}

#[tokio::test]
async fn a_cached_list_is_used_without_a_request_and_other_credentials_miss() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let url = format!("{base}/v1/models");
    let cache = Recorder::default();
    let list = decode_compatible_model_list(ONE_MODEL.as_bytes()).unwrap();
    cache
        .held
        .lock()
        .unwrap()
        .insert(cache_key(&url, &auth("Bearer a")), list);
    let hit = discover(&cache, &base, &auth("Bearer a")).await;
    assert_eq!(hit.unwrap(), limits(Some(4096), Some(4096), Some(1024)));
    assert!(cache.stored.lock().unwrap().is_empty());
    let other = auth("Bearer b");
    let (miss, request) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(
            discover(&cache, &base, &other),
            serve(&listener, 200, "", br#"{"data":[]}"#)
        )
    })
    .await
    .unwrap();
    assert_eq!(miss.unwrap(), TokenLimits::default());
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer b")
    );
    assert_eq!(cache.stored.lock().unwrap().len(), 1);
}

#[rstest]
#[case::bare("http://127.0.0.1:1", "http://127.0.0.1:1/v1/models")]
#[case::ending_in_v1("http://127.0.0.1:1/v1", "http://127.0.0.1:1/v1/models")]
#[case::prefixed_path("http://127.0.0.1:1/api", "http://127.0.0.1:1/api/v1/models")]
#[tokio::test]
async fn the_models_url_joins_the_base_with_path_overlap(#[case] base: &str, #[case] url: &str) {
    let cache = Recorder::default();
    let headers = Headers::new();
    discover(&cache, base, &headers).await.unwrap();
    assert_eq!(cache.stored.lock().unwrap()[0].0, cache_key(url, &headers));
}

#[tokio::test]
async fn a_base_that_is_not_a_url_is_the_only_error() {
    let cache = Recorder::default();
    let error = discover(&cache, "not a url", &Headers::new())
        .await
        .unwrap_err();
    assert!(matches!(error, ModelInfoError::Endpoint(_)));
    assert!(cache.stored.lock().unwrap().is_empty());
}
