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
use super::response::{ImageResponse, ProcessError, process_image_response};

const PNG: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0dIHDR";
const TEN_MINUTES: Duration = Duration::from_mins(10);

fn stream_of(body: &[u8]) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        body.chunks(8192).map(|chunk| Ok(chunk.to_vec())).collect();
    stream::iter(chunks).boxed()
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
#[case::content_length_not_an_integer("content_length_not_an_integer")]
#[case::body_one_byte_over_limit("body_one_byte_over_limit")]
#[case::body_over_limit("body_over_limit")]
#[case::body_at_the_limit("body_at_the_limit")]
#[case::png_bytes("png_bytes")]
#[case::content_type_header_wins("content_type_header_wins")]
#[case::unknown_media_type("unknown_media_type")]
#[case::cache_stores_the_data_url("cache_stores_the_data_url")]
#[case::cache_skips_one_mebibyte("cache_skips_one_mebibyte")]
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
        body: Vec<u8>,
        delay: Duration,
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
        Some(Reply::Delayed { body, delay }) => write_delayed(stream, body, *delay).await,
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

async fn write_delayed(stream: &mut TcpStream, body: &[u8], delay: Duration) -> io::Result<()> {
    sleep(delay).await;
    write_status(stream, 200, None, body).await
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
#[case::zero_limit("zero_limit")]
#[case::cache_hit("cache_hit")]
#[case::connection_refused_three_tries("connection_refused")]
#[case::status_error_not_retried("status_not_retried")]
#[case::ssrf_rejected_before_a_request("ssrf_rejected")]
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
            assert!(server.max_in_flight() <= 20);
            assert!(server.max_in_flight() > 1);
            assert_eq!(server.request_count(), 40);
        }
        "cancelled" => {
            let routes = (0..25_u32)
                .map(|index| {
                    let reply = if index == 0 {
                        Reply::not_found()
                    } else {
                        Reply::Delayed {
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
                    urls[0]
                )
            );
        }
        other => panic!("unknown scenario {other}"),
    }
}
