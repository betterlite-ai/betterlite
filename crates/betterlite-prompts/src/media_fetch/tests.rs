#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use futures_util::StreamExt as _;
use futures_util::stream;
use futures_util::stream::BoxStream;
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use super::cache::ImageCache;
use super::fetcher::Fetcher;
use super::limits::transport_limits;
use super::response::{ImageResponse, ProcessError, process_image_response};

const PNG: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0dIHDR";
const TEN_MINUTES: Duration = Duration::from_mins(10);

fn stream_of(body: &[u8]) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        body.chunks(8192).map(|chunk| Ok(chunk.to_vec())).collect();
    stream::iter(chunks).boxed()
}

fn stream_with_error(
    body: &[u8],
    error: HttpError,
) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let chunks: Vec<Result<Vec<u8>, HttpError>> = body
        .chunks(8192)
        .map(|chunk| Ok(chunk.to_vec()))
        .chain([Err(error)])
        .collect();
    stream::iter(chunks).boxed()
}

fn single_chunk(body: Vec<u8>) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    stream::iter(vec![Ok(body)]).boxed()
}

fn headers_of(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::try_from(*name).unwrap(),
            HeaderValue::try_from(*value).unwrap(),
        );
    }
    headers
}

fn new_cache() -> ImageCache {
    ImageCache::new(Arc::new(|| 0.0_f64)).unwrap()
}

fn image(status: u16, headers: &[(&str, &str)], body: &[u8]) -> ImageResponse {
    ImageResponse {
        status,
        headers: headers_of(headers),
        body: stream_of(body),
    }
}

async fn reading(
    cache: &ImageCache,
    response: ImageResponse,
    url: &str,
    limit: f64,
) -> Result<String, ProcessError> {
    process_image_response(response, url, limit, cache).await
}

#[rstest]
#[case::status_404("status_404")]
#[case::status_204("status_204")]
#[case::content_length_over_limit("content_length_over_limit")]
#[case::content_length_at_the_limit("content_length_at_the_limit")]
#[case::content_length_fractional("content_length_fractional")]
#[case::content_length_not_an_integer("content_length_not_an_integer")]
#[case::body_one_byte_over_limit("body_one_byte_over_limit")]
#[case::body_over_limit("body_over_limit")]
#[case::body_at_the_limit("body_at_the_limit")]
#[case::png_bytes("png_bytes")]
#[case::content_type_header_wins("content_type_header_wins")]
#[case::unknown_media_type("unknown_media_type")]
#[case::cache_stores_the_data_url("cache_stores_the_data_url")]
#[case::cache_skips_one_mebibyte("cache_skips_one_mebibyte")]
#[case::cache_skips_exactly_one_mebibyte("cache_skips_exactly_one_mebibyte")]
#[case::tiny_limit_prints_the_python_float("tiny_limit_prints_the_python_float")]
#[case::body_error_after_the_block_past_the_limit("body_error_after_the_block_past_the_limit")]
#[case::body_in_one_chunk("body_in_one_chunk")]
#[case::negative_limit_empty_body("negative_limit_empty_body")]
#[tokio::test]
async fn response_reading_matches_litellm(#[case] scenario: &str) {
    let cache = new_cache();
    let url = "https://example.com/image";
    match scenario {
        "status_404" => {
            let error = reading(&cache, image(404, &[], b""), url, 50.0)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Unable to fetch image from URL. Status code: 404, url=https://example.com/image"
            );
        }
        "status_204" => {
            let error = reading(&cache, image(204, &[], b""), url, 50.0)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Unable to fetch image from URL. Status code: 204, url=https://example.com/image"
            );
        }
        "content_length_over_limit" => {
            let error = reading(
                &cache,
                image(200, &[("Content-Length", "62914560")], b""),
                url,
                50.0,
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Image size (60.00MB) exceeds maximum allowed size (50.0MB). url=https://example.com/image"
            );
        }
        "content_length_not_an_integer" => {
            let error = reading(
                &cache,
                image(200, &[("Content-Length", "sixty")], b""),
                url,
                50.0,
            )
            .await
            .unwrap_err();
            assert!(matches!(error, ProcessError::ContentLength));
        }
        "body_one_byte_over_limit" => {
            let body = vec![0xAB_u8; 52_428_801];
            let error = reading(&cache, image(200, &[], &body), url, 50.0)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Image size (50.00MB) exceeds maximum allowed size (50.0MB). url=https://example.com/image"
            );
        }
        "body_over_limit" => {
            let body = vec![0xAB_u8; 60_000_000];
            let error = reading(&cache, image(200, &[], &body), url, 50.0)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Image size (50.01MB) exceeds maximum allowed size (50.0MB). url=https://example.com/image"
            );
        }
        "body_at_the_limit" => {
            let body = vec![0xAB_u8; 0x0008_0000];
            let data_url = reading(
                &cache,
                image(200, &[("Content-Type", "image/png")], &body),
                url,
                0.5,
            )
            .await
            .unwrap();
            assert_eq!(
                data_url,
                format!("data:image/png;base64,{}", BASE64.encode(&body))
            );
        }
        "png_bytes" => {
            let data_url = reading(&cache, image(200, &[], PNG), url, 50.0)
                .await
                .unwrap();
            assert_eq!(
                data_url,
                format!("data:image/png;base64,{}", BASE64.encode(PNG))
            );
        }
        "content_type_header_wins" => {
            let data_url = reading(
                &cache,
                image(200, &[("Content-Type", "image/gif")], PNG),
                "https://example.com/image.png",
                50.0,
            )
            .await
            .unwrap();
            assert_eq!(
                data_url,
                format!("data:image/gif;base64,{}", BASE64.encode(PNG))
            );
        }
        "unknown_media_type" => {
            let error = reading(&cache, image(200, &[], b"hello"), url, 50.0)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Unable to determine image content type from the server's headers, the URL, or the image bytes. url=https://example.com/image"
            );
        }
        "cache_stores_the_data_url" => {
            let data_url = reading(
                &cache,
                image(200, &[("Content-Type", "image/png")], b"tiny"),
                url,
                50.0,
            )
            .await
            .unwrap();
            assert_eq!(cache.get(url), Some(data_url));
        }
        "cache_skips_one_mebibyte" => {
            let body = vec![0xAB_u8; 0x000c_0000];
            let data_url = reading(
                &cache,
                image(200, &[("Content-Type", "image/png")], &body),
                url,
                2.0,
            )
            .await
            .unwrap();
            assert!(data_url.len() >= 0x0010_0000);
            assert_eq!(cache.get(url), None);
        }
        "content_length_at_the_limit" => {
            let data_url = reading(
                &cache,
                image(
                    200,
                    &[
                        ("Content-Length", "52428800"),
                        ("Content-Type", "image/png"),
                    ],
                    b"tiny",
                ),
                url,
                50.0,
            )
            .await
            .unwrap();
            assert_eq!(
                data_url,
                format!("data:image/png;base64,{}", BASE64.encode(b"tiny"))
            );
        }
        "content_length_fractional" => {
            let error = reading(
                &cache,
                image(200, &[("Content-Length", "50.5")], b""),
                url,
                50.0,
            )
            .await
            .unwrap_err();
            assert!(matches!(error, ProcessError::ContentLength));
        }
        "cache_skips_exactly_one_mebibyte" => {
            let body = vec![0xAB_u8; 786_418];
            let data_url = reading(
                &cache,
                image(200, &[("Content-Type", "a/b")], &body),
                url,
                2.0,
            )
            .await
            .unwrap();
            assert_eq!(data_url.len(), 0x0010_0000);
            assert_eq!(cache.get(url), None);
        }
        "tiny_limit_prints_the_python_float" => {
            let body = vec![0xAB_u8; 1000];
            let error = reading(&cache, image(200, &[], &body), url, 1e-5)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Image size (0.00MB) exceeds maximum allowed size (1e-05MB). url=https://example.com/image"
            );
        }
        "body_error_after_the_block_past_the_limit" => {
            let body = vec![0xAB_u8; 0x0008_2000];
            let response = ImageResponse {
                status: 200,
                headers: headers_of(&[]),
                body: stream_with_error(&body, HttpError::Timeout),
            };
            let error = reading(&cache, response, url, 0.5).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Image size (0.51MB) exceeds maximum allowed size (0.5MB). url=https://example.com/image"
            );
        }
        "body_in_one_chunk" => {
            let response = ImageResponse {
                status: 200,
                headers: headers_of(&[]),
                body: single_chunk(vec![0xAB_u8; 600_000]),
            };
            let error = reading(&cache, response, url, 0.5).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Image size (0.51MB) exceeds maximum allowed size (0.5MB). url=https://example.com/image"
            );
        }
        "negative_limit_empty_body" => {
            let data_url = reading(
                &cache,
                image(200, &[("Content-Type", "image/png")], b""),
                url,
                -1.0,
            )
            .await
            .unwrap();
            assert_eq!(data_url, "data:image/png;base64,");
        }
        other => panic!("unknown scenario {other}"),
    }
}

enum Reply {
    Status {
        status: u16,
        content_type: Option<String>,
        body: Vec<u8>,
    },
    Delayed {
        status: u16,
        body: Vec<u8>,
        delay: Duration,
    },
    CloseDelimited {
        body: Vec<u8>,
    },
    Drip {
        interval: Duration,
    },
}

impl Reply {
    fn ok(body: &[u8]) -> Self {
        Self::Status {
            status: 200,
            content_type: None,
            body: body.to_vec(),
        }
    }

    fn not_found() -> Self {
        Self::Status {
            status: 404,
            content_type: None,
            body: Vec::new(),
        }
    }
}

struct Server {
    address: SocketAddr,
    state: Arc<State>,
    tasks: JoinSet<()>,
}

#[derive(Default)]
struct State {
    requests: Mutex<Vec<String>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl State {
    fn note_request(&self, path: &str) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.to_owned());
        let now = self
            .in_flight
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }
}

impl Server {
    async fn start(route_table: Vec<(String, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(State::default());
        let routes: Arc<BTreeMap<String, Reply>> = Arc::new(route_table.into_iter().collect());
        let mut tasks = JoinSet::new();
        let accept_state = Arc::clone(&state);
        tasks.spawn(async move {
            let mut open = JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                open.spawn(converse(
                    stream,
                    Arc::clone(&routes),
                    Arc::clone(&accept_state),
                ));
            }
        });
        Self {
            address,
            state,
            tasks,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn request_count(&self) -> usize {
        self.state.request_count()
    }

    fn max_in_flight(&self) -> usize {
        self.state.max_in_flight()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn converse(mut stream: TcpStream, routes: Arc<BTreeMap<String, Reply>>, state: Arc<State>) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk).await {
                Ok(count) if count > 0 => buffer.extend_from_slice(chunk.get(..count).unwrap()),
                _ => return,
            }
        }
        let end = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            .saturating_add(4);
        let head = String::from_utf8(buffer.drain(..end).collect()).unwrap();
        let path = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_owned();
        let route = path.split('?').next().unwrap_or("/").to_owned();
        state.note_request(&route);
        let reply = routes.get(&route);
        if write_reply(&mut stream, reply).await.is_err() {
            state.in_flight.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        state.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn write_reply(stream: &mut TcpStream, reply: Option<&Reply>) -> io::Result<()> {
    match reply {
        Some(Reply::Status {
            status,
            content_type,
            body,
        }) => write_status(stream, *status, content_type.as_deref(), body).await,
        Some(Reply::Delayed {
            status,
            body,
            delay,
        }) => write_delayed(stream, *status, body, *delay).await,
        Some(Reply::CloseDelimited { body }) => write_close_delimited(stream, body).await,
        Some(Reply::Drip { interval }) => write_drip(stream, *interval).await,
        None => write_status(stream, 404, None, b"").await,
    }
}

async fn write_status(
    stream: &mut TcpStream,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(declared) = content_type {
        write!(head, "Content-Type: {declared}\r\n").unwrap();
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await
}

async fn write_delayed(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
    delay: Duration,
) -> io::Result<()> {
    sleep(delay).await;
    write_status(stream, status, None, body).await
}

async fn write_close_delimited(stream: &mut TcpStream, body: &[u8]) -> io::Result<()> {
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
        .await?;
    stream.write_all(body).await
}

async fn write_drip(stream: &mut TcpStream, interval: Duration) -> io::Result<()> {
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n")
        .await?;
    for _ in 0..10_u32 {
        stream.write_all(b"x").await?;
        stream.flush().await?;
        sleep(interval).await;
    }
    Ok(())
}

fn new_client(read_timeout: Duration) -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let limits = TransportLimits {
        read_timeout,
        ..TransportLimits::DEFAULT
    };
    ProxiedClient::build(base_builder(), &policy, limits).unwrap()
}

fn new_fetcher(allowed: &[&str], limit: f64, read_timeout: Duration) -> Fetcher<SystemResolver> {
    let allowed_hosts: Vec<String> = allowed.iter().map(|host| (*host).to_owned()).collect();
    let validator = UrlValidator::new(SystemResolver, &allowed_hosts, &Verification::Enabled);
    let urls = UserUrlFetcher {
        validator,
        validation: true,
    };
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 0.0_f64);
    Fetcher::new(new_client(read_timeout), urls, limit, clock).unwrap()
}

async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[rstest]
#[case::data_url_passthrough("data_url_passthrough")]
#[case::data_url_without_base64_is_not_passed_through("data_without_base64")]
#[case::zero_limit("zero_limit")]
#[case::cache_hit("cache_hit")]
#[case::connection_refused_three_tries("connection_refused")]
#[case::status_error_not_retried("status_not_retried")]
#[case::body_over_limit_is_not_retried("body_over_limit_not_retried")]
#[case::ssrf_rejected_before_a_request("ssrf_rejected")]
#[case::infinite_limit_retried("infinite_limit")]
#[case::negative_timeout_fails_every_try("negative_timeout")]
#[case::total_deadline("total_deadline")]
#[tokio::test]
async fn url_conversion_matches_litellm(#[case] scenario: &str) {
    match scenario {
        "data_url_passthrough" => {
            let fetcher = new_fetcher(&[], 50.0, TEN_MINUTES);
            assert_eq!(
                fetcher.convert("data:image/png;base64,AAAA").await.unwrap(),
                "data:image/png;base64,AAAA"
            );
        }
        "data_without_base64" => {
            let fetcher = new_fetcher(&[], 50.0, TEN_MINUTES);
            let error = fetcher.convert("data:image/png,raw").await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "Error: Unable to fetch image from URL. The proxy could not resolve this host or its URL policy rejected it; an admin can check the proxy log and `user_url_allowed_hosts` in litellm_settings. url=data:image/png,raw"
            );
        }
        "zero_limit" => {
            let server = Server::start(vec![("/image.png".to_owned(), Reply::ok(b"body"))]).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 0.0, TEN_MINUTES);
            let url = server.url("/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Error: Image URL download is disabled (MAX_IMAGE_URL_DOWNLOAD_SIZE_MB=0). url={url}"
                )
            );
            assert_eq!(server.request_count(), 0);
        }
        "cache_hit" => {
            let server =
                Server::start(vec![("/image.png".to_owned(), Reply::ok(b"png-body"))]).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 50.0, TEN_MINUTES);
            let url = server.url("/image.png");
            let first = fetcher.convert(&url).await.unwrap();
            let second = fetcher.convert(&url).await.unwrap();
            assert_eq!(first, second);
            assert_eq!(
                first,
                format!("data:image/png;base64,{}", BASE64.encode(b"png-body"))
            );
            assert_eq!(server.request_count(), 1);
        }
        "connection_refused" => {
            let port = closed_port().await;
            let fetcher = new_fetcher(&["127.0.0.1"], 50.0, TEN_MINUTES);
            let url = format!("http://127.0.0.1:{port}/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Error: Unable to fetch image from URL after 3 attempts. url={url}")
            );
        }
        "status_not_retried" => {
            let server = Server::start(vec![("/image.png".to_owned(), Reply::not_found())]).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 50.0, TEN_MINUTES);
            let url = server.url("/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Error: Unable to fetch image from URL. Status code: 404, url={url}")
            );
            assert_eq!(server.request_count(), 1);
        }
        "body_over_limit_not_retried" => {
            // no Content-Length, so the declared-size check cannot answer before the body count does
            let reply = Reply::CloseDelimited {
                body: vec![0xAB_u8; 600_000],
            };
            let server = Server::start(vec![("/image.png".to_owned(), reply)]).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 0.5, TEN_MINUTES);
            let url = server.url("/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Error: Image size (0.51MB) exceeds maximum allowed size (0.5MB). url={url}"
                )
            );
            assert_eq!(server.request_count(), 1);
        }
        "ssrf_rejected" => {
            let server = Server::start(vec![("/image.png".to_owned(), Reply::ok(b"body"))]).await;
            let fetcher = new_fetcher(&[], 50.0, TEN_MINUTES);
            let url = server.url("/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Error: Unable to fetch image from URL. The proxy could not resolve this host or its URL policy rejected it; an admin can check the proxy log and `user_url_allowed_hosts` in litellm_settings. url={url}"
                )
            );
            assert_eq!(server.request_count(), 0);
        }
        "infinite_limit" => {
            let server = Server::start(vec![("/image.png".to_owned(), Reply::ok(b"body"))]).await;
            let fetcher = new_fetcher(&["127.0.0.1"], f64::INFINITY, TEN_MINUTES);
            let url = server.url("/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Error: Unable to fetch image from URL after 3 attempts. url={url}")
            );
            assert_eq!(server.request_count(), 3);
        }
        "negative_timeout" => {
            let server = Server::start(vec![("/image.png".to_owned(), Reply::ok(b"body"))]).await;
            let core = CoreSettings {
                request_timeout_seconds: -5.0_f64,
                request_timeout_explicit: true,
                litellm_request_timeout: None,
                model_timeout_overrides: Vec::new(),
            };
            let policy =
                Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
            let client =
                ProxiedClient::build(base_builder(), &policy, transport_limits(&core)).unwrap();
            let validator = UrlValidator::new(
                SystemResolver,
                &["127.0.0.1".to_owned()],
                &Verification::Enabled,
            );
            let urls = UserUrlFetcher {
                validator,
                validation: true,
            };
            let fetcher = Fetcher::new(client, urls, 50.0, Arc::new(|| 0.0_f64)).unwrap();
            let url = server.url("/image.png");
            let error = fetcher.convert(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Error: Unable to fetch image from URL after 3 attempts. url={url}")
            );
        }
        "total_deadline" => {
            let routes = vec![(
                "/drip.png".to_owned(),
                Reply::Drip {
                    interval: Duration::from_millis(50),
                },
            )];
            let server = Server::start(routes).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 50.0, Duration::from_millis(200));
            let url = server.url("/drip.png");
            let outcome = timeout(Duration::from_secs(5), fetcher.convert(&url))
                .await
                .expect("a try without a total deadline would never end");
            assert_eq!(
                outcome.unwrap_err().to_string(),
                format!("Error: Unable to fetch image from URL after 3 attempts. url={url}")
            );
        }
        other => panic!("unknown scenario {other}"),
    }
}

#[rstest]
#[case::at_most_twenty_in_flight("bounded")]
#[case::first_failure_cancels_the_rest("cancelled")]
#[tokio::test]
async fn parallel_fetches_are_bounded(#[case] scenario: &str) {
    match scenario {
        "bounded" => {
            let routes = (0..40_u32)
                .map(|index| {
                    let reply = Reply::Delayed {
                        status: 200,
                        body: index.to_string().into_bytes(),
                        delay: Duration::from_millis(
                            u64::from(40_u32.saturating_sub(index))
                                .saturating_mul(5)
                                .saturating_add(100),
                        ),
                    };
                    (format!("/{index}.png"), reply)
                })
                .collect();
            let server = Server::start(routes).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 50.0, TEN_MINUTES);
            let urls: Vec<String> = (0..40_u32)
                .map(|index| server.url(&format!("/{index}.png")))
                .collect();
            let data_urls = fetcher.fetch_all(&urls).await.unwrap();
            assert_eq!(data_urls.len(), 40);
            let decoded: Vec<String> = data_urls
                .iter()
                .map(|data_url| {
                    let encoded = data_url.split_once(";base64,").unwrap().1;
                    String::from_utf8(BASE64.decode(encoded).unwrap()).unwrap()
                })
                .collect();
            let expected: Vec<String> = (0..40_u32).map(|index| index.to_string()).collect();
            assert_eq!(decoded, expected);
            assert_eq!(server.max_in_flight(), 20);
            assert_eq!(server.request_count(), 40);
        }
        "cancelled" => {
            // the failing URL sits behind 3-second URLs, so in-order buffering would miss the 2-second bound
            let routes = (0..25_u32)
                .map(|index| {
                    let reply = if index == 10 {
                        Reply::Delayed {
                            status: 404,
                            body: Vec::new(),
                            delay: Duration::from_millis(250),
                        }
                    } else {
                        Reply::Delayed {
                            status: 200,
                            body: index.to_string().into_bytes(),
                            delay: Duration::from_secs(3),
                        }
                    };
                    (format!("/{index}.png"), reply)
                })
                .collect();
            let server = Server::start(routes).await;
            let fetcher = new_fetcher(&["127.0.0.1"], 50.0, TEN_MINUTES);
            let urls: Vec<String> = (0..25_u32)
                .map(|index| server.url(&format!("/{index}.png")))
                .collect();
            let outcome = timeout(Duration::from_secs(2), fetcher.fetch_all(&urls))
                .await
                .expect("the first failure cancels the fetches still in flight");
            let error = outcome.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Error: Unable to fetch image from URL. Status code: 404, url={}",
                    urls[10]
                )
            );
            sleep(Duration::from_millis(200)).await;
            assert_eq!(server.request_count(), 20);
        }
        other => panic!("unknown scenario {other}"),
    }
}

#[rstest]
#[case::unset(None, 6000.0_f64, 6000.0_f64)]
#[case::request_timeout_variable(None, 12.0_f64, 12.0_f64)]
#[case::litellm_settings_wins(Some(4.0_f64), 12.0_f64, 4.0_f64)]
fn fetch_timeout_follows_litellm(
    #[case] litellm_request_timeout: Option<f64>,
    #[case] request_timeout_seconds: f64,
    #[case] expected_seconds: f64,
) {
    let core = CoreSettings {
        request_timeout_seconds,
        request_timeout_explicit: false,
        litellm_request_timeout,
        model_timeout_overrides: Vec::new(),
    };
    let limits = transport_limits(&core);
    let expected = Duration::try_from_secs_f64(expected_seconds).unwrap();
    assert_eq!(limits.connect_timeout, expected);
    assert_eq!(limits.read_timeout, expected);
}

#[rstest]
#[case::negative_fails_every_try(-5.0_f64, Duration::ZERO)]
#[case::infinite_fails_every_try(f64::INFINITY, Duration::ZERO)]
#[case::huge_is_no_timeout(1e20_f64, Duration::MAX)]
#[case::zero_is_no_timeout(0.0_f64, Duration::MAX)]
fn extreme_fetch_timeouts_follow_litellm(#[case] seconds: f64, #[case] expected: Duration) {
    let core = CoreSettings {
        request_timeout_seconds: seconds,
        request_timeout_explicit: true,
        litellm_request_timeout: None,
        model_timeout_overrides: Vec::new(),
    };
    let limits = transport_limits(&core);
    assert_eq!(limits.connect_timeout, expected);
    assert_eq!(limits.read_timeout, expected);
}

fn movable_cache() -> (Arc<Mutex<f64>>, ImageCache) {
    let now = Arc::new(Mutex::new(0.0_f64));
    let clock = Arc::clone(&now);
    let cache = ImageCache::new(Arc::new(move || *clock.lock().unwrap())).unwrap();
    (now, cache)
}

fn store_every(cache: &ImageCache, urls: &[String]) {
    for url in urls {
        cache.store(url, "data:image/png;base64,AAAA");
    }
}

#[rstest]
#[case::expiry_at_600_seconds("expiry")]
#[case::capacity_keeps_ten("capacity")]
#[case::item_just_under_one_mebibyte("item")]
fn image_cache_limits_follow_litellm(#[case] scenario: &str) {
    let (now, cache) = movable_cache();
    match scenario {
        "expiry" => {
            let entry = "https://example.com/a.png";
            cache.store(entry, "data:image/png;base64,AAAA");
            *now.lock().unwrap() = 600.0_f64;
            assert_eq!(
                cache.get(entry),
                Some("data:image/png;base64,AAAA".to_owned())
            );
            *now.lock().unwrap() = 600.5_f64;
            assert_eq!(cache.get(entry), None);
        }
        "capacity" => {
            let urls: Vec<String> = (0..11_u32)
                .map(|index| format!("https://example.com/{index}.png"))
                .collect();
            store_every(&cache, &urls);
            let kept = urls
                .iter()
                .filter(|entry| cache.get(entry.as_str()).is_some())
                .count();
            assert_eq!(kept, 10);
            assert!(cache.get(&urls[10]).is_some());
        }
        "item" => {
            let entry = "https://example.com/a.png";
            let value = "A".repeat(0x000f_ffff);
            cache.store(entry, &value);
            assert_eq!(cache.get(entry), Some(value));
        }
        other => panic!("unknown scenario {other}"),
    }
}
