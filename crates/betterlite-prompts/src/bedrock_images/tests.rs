#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;
use std::net::SocketAddr;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::bedrock_images::block::create_bedrock_block;
use crate::bedrock_images::error::BedrockImageError;
use crate::bedrock_images::formats::validate_format;
use crate::bedrock_images::parse::{parse_content_type, parse_mime_type};
use crate::bedrock_images::processor::{Base64Image, parse_base64_image, process_image};
use crate::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS, post_call_image_processing};
use crate::bedrock_images::types::ContentBlock;
use crate::media_fetch::response::BodyReadError;

const PNG: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0dIHDR";
const EXAMPLE_URL: &str = "https://example.com/image";

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    truncate_to: Option<usize>,
    drip: bool,
}

impl Reply {
    fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            truncate_to: None,
            drip: false,
        }
    }

    fn image(content_type: &str, body: &[u8]) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".to_owned(), content_type.to_owned())],
            body: body.to_vec(),
            truncate_to: None,
            drip: false,
        }
    }
}

struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    tasks: JoinSet<()>,
}

impl Server {
    async fn start(routes: Vec<(String, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(routes.into_iter().collect::<BTreeMap<String, Reply>>());
        let state = Arc::clone(&requests);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let mut open = JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                open.spawn(serve(stream, Arc::clone(&shared), Arc::clone(&state)));
            }
        });
        Self {
            address,
            requests,
            tasks,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn serve(
    mut stream: TcpStream,
    routes: Arc<BTreeMap<String, Reply>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let Some(path) = request_path(&mut stream).await else {
        return;
    };
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(path.clone());
    let missing = Reply::status(404);
    let reply = routes.get(&path).unwrap_or(&missing);
    let _outcome = write_reply(&mut stream, reply).await;
}

async fn request_path(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).await.ok()?;
        if count == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..count)?);
    }
    let head = str::from_utf8(&buffer).ok()?;
    head.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(str::to_owned)
}

async fn write_reply(stream: &mut TcpStream, reply: &Reply) -> io::Result<()> {
    let mut response = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        write!(response, "{name}: {value}\r\n").unwrap();
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await?;
    if reply.drip {
        for _ in 0..20_u32 {
            stream.write_all(b"x").await?;
            stream.flush().await?;
            sleep(Duration::from_millis(50)).await;
        }
        return Ok(());
    }
    if let Some(limit) = reply.truncate_to {
        let partial = reply.body.get(..limit).unwrap_or(&reply.body);
        stream.write_all(partial).await?;
        return Ok(());
    }
    stream.write_all(&reply.body).await
}

fn route(path: &str, reply: Reply) -> (String, Reply) {
    (path.to_owned(), reply)
}

fn remote_with(
    limits: TransportLimits,
    allowed: &[&str],
    max_mb: f64,
    validation: bool,
) -> RemoteImage<SystemResolver> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::build(base_builder(), &policy, limits).unwrap();
    let hosts: Vec<String> = allowed.iter().map(|host| (*host).to_owned()).collect();
    let validator = UrlValidator::new(SystemResolver, &hosts, &Verification::Enabled);
    RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation,
        },
        deadline: limits.read_timeout,
        max_download_size_mb: max_mb,
    }
}

fn remote(allowed: &[&str], max_mb: f64) -> RemoteImage<SystemResolver> {
    remote_with(TRANSPORT_LIMITS, allowed, max_mb, true)
}

fn assert_video_block(format: &str) {
    assert_eq!(
        serde_json::to_string(&create_bedrock_block(
            "QUJD",
            &format!("video/{format}"),
            format
        ))
        .unwrap(),
        format!(r#"{{"video":{{"format":"{format}","source":{{"bytes":"QUJD"}}}}}}"#)
    );
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

#[rstest]
#[case::content_type_keeps_the_media_type("content_type_keeps_the_media_type")]
#[case::content_type_defaults_to_text_plain("content_type_defaults_to_text_plain")]
#[case::mime_type_reads_the_data_prefix("mime_type_reads_the_data_prefix")]
#[case::mime_type_needs_the_exact_marker("mime_type_needs_the_exact_marker")]
#[case::base64_image_splits_metadata_and_payload("base64_image_splits_metadata_and_payload")]
#[case::base64_image_reads_parameters("base64_image_reads_parameters")]
#[case::base64_image_defaults_to_jpeg("base64_image_defaults_to_jpeg")]
#[case::base64_image_rejects_extra_commas("base64_image_rejects_extra_commas")]
#[case::base64_image_requires_a_media_subtype("base64_image_requires_a_media_subtype")]
#[case::format_validation_accepts_images_and_videos("format_validation_accepts_images_and_videos")]
#[case::format_validation_rejects_unknown_image_formats(
    "format_validation_rejects_unknown_image_formats"
)]
#[case::format_validation_maps_document_mime_types("format_validation_maps_document_mime_types")]
#[case::format_validation_rejects_unknown_documents("format_validation_rejects_unknown_documents")]
#[case::video_blocks_wrap_every_supported_format("video_blocks_wrap_every_supported_format")]
#[case::blocks_wrap_image_video_and_document_content(
    "blocks_wrap_image_video_and_document_content"
)]
#[case::document_blocks_hash_the_normalized_payload("document_blocks_hash_the_normalized_payload")]
#[case::post_call_takes_the_header_media_type("post_call_takes_the_header_media_type")]
#[case::post_call_infers_the_media_type_from_the_url(
    "post_call_infers_the_media_type_from_the_url"
)]
#[case::post_call_sniffs_the_media_type_from_the_bytes(
    "post_call_sniffs_the_media_type_from_the_bytes"
)]
#[case::post_call_fails_with_an_unknown_media_type("post_call_fails_with_an_unknown_media_type")]
#[case::transport_limits_match_litellm_defaults("transport_limits_match_litellm_defaults")]
#[case::remote_fetch_returns_base64_and_content_type(
    "remote_fetch_returns_base64_and_content_type"
)]
#[case::remote_fetch_fails_with_httpx_status_text("remote_fetch_fails_with_httpx_status_text")]
#[case::remote_fetch_fails_with_informational_status_text(
    "remote_fetch_fails_with_informational_status_text"
)]
#[case::remote_fetch_fails_with_server_error_text("remote_fetch_fails_with_server_error_text")]
#[case::remote_fetch_fails_with_invalid_status_text("remote_fetch_fails_with_invalid_status_text")]
#[case::remote_fetch_fails_with_redirect_status_text(
    "remote_fetch_fails_with_redirect_status_text"
)]
#[case::remote_fetch_names_the_last_request_url("remote_fetch_names_the_last_request_url")]
#[case::remote_fetch_fails_a_body_over_the_limit("remote_fetch_fails_a_body_over_the_limit")]
#[case::remote_fetch_rejects_a_blocked_loopback("remote_fetch_rejects_a_blocked_loopback")]
#[case::remote_fetch_fails_a_truncated_body("remote_fetch_fails_a_truncated_body")]
#[case::remote_fetch_ends_a_dripping_body_at_the_deadline(
    "remote_fetch_ends_a_dripping_body_at_the_deadline"
)]
#[case::remote_fetch_ends_a_dripping_body_at_the_total_deadline(
    "remote_fetch_ends_a_dripping_body_at_the_total_deadline"
)]
#[case::process_dispatches_base64_before_a_url("process_dispatches_base64_before_a_url")]
#[case::process_rejects_an_unsupported_image_url("process_rejects_an_unsupported_image_url")]
#[case::process_overrides_the_media_type_and_format("process_overrides_the_media_type_and_format")]
#[case::process_override_sets_the_mime_type("process_override_sets_the_mime_type")]
#[case::process_override_without_a_subtype_fails("process_override_without_a_subtype_fails")]
#[case::process_fetches_a_remote_image("process_fetches_a_remote_image")]
#[case::process_rejects_an_unsupported_format("process_rejects_an_unsupported_format")]
#[case::process_returns_a_document_block("process_returns_a_document_block")]
#[case::process_uses_the_fetched_content_type("process_uses_the_fetched_content_type")]
#[tokio::test]
async fn bedrock_images_behavior_tests_behavior(#[case] scenario: &str) {
    match scenario {
        "content_type_keeps_the_media_type" => {
            assert_eq!(parse_content_type("Image/PNG; charset=utf-8"), "image/png");
            assert_eq!(parse_content_type(" image/png ; x=1"), "image/png");
            assert_eq!(parse_content_type("image/png;"), "image/png");
        }
        "content_type_defaults_to_text_plain" => {
            assert_eq!(parse_content_type("image"), "text/plain");
            assert_eq!(parse_content_type(""), "text/plain");
            assert_eq!(parse_content_type("image/png/extra"), "text/plain");
        }
        "mime_type_reads_the_data_prefix" => {
            assert_eq!(
                parse_mime_type("data:image/png;base64,AAAA"),
                Some("image/png")
            );
            assert_eq!(
                parse_mime_type("data:image/png;charset=x;base64,AAAA"),
                Some("image/png;charset=x")
            );
            assert_eq!(parse_mime_type("data:;base64,AAAA"), Some(""));
        }
        "mime_type_needs_the_exact_marker" => {
            assert_eq!(parse_mime_type("image/png;base64,AAAA"), None);
            assert_eq!(parse_mime_type("data:image/png,AAAA"), None);
            assert_eq!(parse_mime_type("data:image/png\n;base64,AAAA"), None);
            assert_eq!(parse_mime_type("DATA:image/png;base64,AAAA"), None);
        }
        "base64_image_splits_metadata_and_payload" => {
            assert_eq!(
                parse_base64_image("data:image/png;base64,AAAA").unwrap(),
                Base64Image {
                    data: "AAAA".to_owned(),
                    mime_type: "image/png".to_owned(),
                    format: "png".to_owned(),
                }
            );
        }
        "base64_image_reads_parameters" => {
            let parsed = parse_base64_image("data:image/gif;charset=utf-8;base64,R0lG").unwrap();
            assert_eq!(parsed.mime_type, "image/gif");
            assert_eq!(parsed.format, "gif");
            assert_eq!(parsed.data, "R0lG");
        }
        "base64_image_defaults_to_jpeg" => {
            assert_eq!(
                parse_base64_image("data:image/png,AAAA").unwrap(),
                Base64Image {
                    data: "AAAA".to_owned(),
                    mime_type: "image/jpeg".to_owned(),
                    format: "jpeg".to_owned(),
                }
            );
        }
        "base64_image_rejects_extra_commas" => {
            assert!(matches!(
                parse_base64_image("data:image/png;base64,AA,AA"),
                Err(BedrockImageError::DataUrlParts)
            ));
            assert!(matches!(
                parse_base64_image("data:image/png;base64"),
                Err(BedrockImageError::DataUrlParts)
            ));
        }
        "base64_image_requires_a_media_subtype" => {
            assert!(matches!(
                parse_base64_image("data:png;base64,AAAA"),
                Err(BedrockImageError::MissingMediaSubtype { .. })
            ));
        }
        "format_validation_accepts_images_and_videos" => {
            assert_eq!(validate_format("image/png", "png").unwrap(), "png");
            assert_eq!(validate_format("image/jpeg", "jpeg").unwrap(), "jpeg");
            assert_eq!(validate_format("image/gif", "gif").unwrap(), "gif");
            assert_eq!(validate_format("image/webp", "webp").unwrap(), "webp");
            assert_eq!(validate_format("video/mp4", "mp4").unwrap(), "mp4");
            assert_eq!(validate_format("video/mov", "mov").unwrap(), "mov");
            assert_eq!(validate_format("video/mkv", "mkv").unwrap(), "mkv");
            assert_eq!(validate_format("video/webm", "webm").unwrap(), "webm");
            assert_eq!(validate_format("video/flv", "flv").unwrap(), "flv");
            assert_eq!(validate_format("video/mpeg", "mpeg").unwrap(), "mpeg");
            assert_eq!(validate_format("video/mpg", "mpg").unwrap(), "mpg");
            assert_eq!(validate_format("video/wmv", "wmv").unwrap(), "wmv");
            assert_eq!(validate_format("video/3gp", "3gp").unwrap(), "3gp");
        }
        "format_validation_rejects_unknown_image_formats" => {
            let error = validate_format("image/tiff", "tiff").unwrap_err();
            assert_eq!(
                error.to_string(),
                "Unsupported image format: tiff. Supported formats: ['mp4', 'mov', 'mkv', 'webm', 'flv', 'mpeg', 'mpg', 'wmv', '3gp', 'png', 'jpeg', 'gif', 'webp']"
            );
        }
        "format_validation_maps_document_mime_types" => {
            assert_eq!(validate_format("application/pdf", "pdf").unwrap(), "pdf");
            assert_eq!(validate_format("application/msword", "doc").unwrap(), "doc");
            assert_eq!(
                validate_format(
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                    "docx"
                )
                .unwrap(),
                "docx"
            );
            assert_eq!(
                validate_format("application/vnd.ms-excel", "xls").unwrap(),
                "xls"
            );
            assert_eq!(
                validate_format(
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                    "xlsx"
                )
                .unwrap(),
                "xlsx"
            );
            assert_eq!(validate_format("text/csv", "csv").unwrap(), "csv");
            assert_eq!(validate_format("text/html", "html").unwrap(), "html");
            assert_eq!(validate_format("text/plain", "plain").unwrap(), "txt");
            assert_eq!(validate_format("text/markdown", "markdown").unwrap(), "md");
            assert_eq!(validate_format("application/PDF", "PDF").unwrap(), "pdf");
        }
        "format_validation_rejects_unknown_documents" => {
            let error = validate_format("application/x-unknown", "unknown").unwrap_err();
            assert_eq!(
                error.to_string(),
                "No supported extensions for MIME type: application/x-unknown. Supported formats: ['pdf', 'csv', 'doc', 'docx', 'xls', 'xlsx', 'html', 'txt', 'md']"
            );
        }
        "video_blocks_wrap_every_supported_format" => {
            assert_video_block("mp4");
            assert_video_block("mov");
            assert_video_block("mkv");
            assert_video_block("webm");
            assert_video_block("flv");
            assert_video_block("mpeg");
            assert_video_block("mpg");
            assert_video_block("wmv");
            assert_video_block("3gp");
        }
        "blocks_wrap_image_video_and_document_content" => {
            let image = create_bedrock_block("QUJD", "image/png", "png");
            assert_eq!(
                serde_json::to_string(&image).unwrap(),
                r#"{"image":{"format":"png","source":{"bytes":"QUJD"}}}"#
            );
            let video = create_bedrock_block("QUJD", "video/mp4", "mp4");
            assert_eq!(
                serde_json::to_string(&video).unwrap(),
                r#"{"video":{"format":"mp4","source":{"bytes":"QUJD"}}}"#
            );
            let document = create_bedrock_block("QUJD", "application/pdf", "pdf");
            let ContentBlock::Document { document: block } = &document else {
                panic!("a document media type answers with a document block");
            };
            assert_eq!(block.format, "pdf");
            assert_eq!(block.source.bytes, "QUJD");
            assert!(block.name.starts_with("DocumentPDFmessages_"));
            assert!(block.name.ends_with("_pdf"));
        }
        "document_blocks_hash_the_normalized_payload" => {
            let whitespace = create_bedrock_block(" QUJD\n", "text/plain", "txt");
            assert_eq!(
                serde_json::to_string(&whitespace).unwrap(),
                r#"{"document":{"format":"txt","source":{"bytes":" QUJD\n"},"name":"DocumentPDFmessages_0a702913308f6ec6_txt"}}"#
            );
            let large_payload = format!("{}{}", "A".repeat(0x0001_0000), "B".repeat(10));
            let sampled = create_bedrock_block(&large_payload, "application/pdf", "pdf");
            assert_eq!(
                serde_json::to_string(&sampled).unwrap(),
                format!(
                    r#"{{"document":{{"format":"pdf","source":{{"bytes":"{large_payload}"}},"name":"DocumentPDFmessages_850366edec6eebdd_pdf"}}}}"#
                )
            );
            let separated = create_bedrock_block("QUJD\x1cR0lG\x1f", "application/pdf", "pdf");
            let ContentBlock::Document { document } = &separated else {
                panic!("a document media type answers with a document block");
            };
            assert_eq!(document.source.bytes, "QUJD\x1cR0lG\x1f");
            assert_eq!(document.name, "DocumentPDFmessages_025874a868cf687d_pdf");
        }
        "post_call_takes_the_header_media_type" => {
            let headers = headers_of(&[("content-type", "image/png; charset=utf-8")]);
            let (base64, content_type) =
                post_call_image_processing("https://example.com/pic.gif", &headers, PNG).unwrap();
            assert_eq!(base64, BASE64.encode(PNG));
            assert_eq!(content_type, "image/png");
        }
        "post_call_infers_the_media_type_from_the_url" => {
            let headers = headers_of(&[("content-type", "application/octet-stream")]);
            let (base64, content_type) =
                post_call_image_processing("https://example.com/pic.png", &headers, b"hello")
                    .unwrap();
            assert_eq!(base64, BASE64.encode(b"hello"));
            assert_eq!(content_type, "image/png");
        }
        "post_call_sniffs_the_media_type_from_the_bytes" => {
            let (base64, content_type) =
                post_call_image_processing(EXAMPLE_URL, &HeaderMap::new(), PNG).unwrap();
            assert_eq!(base64, BASE64.encode(PNG));
            assert_eq!(content_type, "image/png");
        }
        "post_call_fails_with_an_unknown_media_type" => {
            let error =
                post_call_image_processing(EXAMPLE_URL, &HeaderMap::new(), b"hello").unwrap_err();
            assert_eq!(
                error.to_string(),
                "could not determine the content type of url https://example.com/image"
            );
        }
        "transport_limits_match_litellm_defaults" => {
            assert_eq!(TRANSPORT_LIMITS.connect_timeout, Duration::from_secs(5));
            assert_eq!(TRANSPORT_LIMITS.read_timeout, Duration::from_secs(600));
        }
        "remote_fetch_returns_base64_and_content_type" => {
            let server = Server::start(vec![route(
                "/image.png",
                Reply::image("image/png; charset=utf-8", PNG),
            )])
            .await;
            let url = server.url("/image.png");
            let (base64, content_type) = remote(&["127.0.0.1"], 50.0)
                .image_details(&url)
                .await
                .unwrap();
            assert_eq!(base64, BASE64.encode(PNG));
            assert_eq!(content_type, "image/png");
            assert_eq!(server.request_count(), 1);
        }
        "remote_fetch_fails_with_httpx_status_text" => {
            let server = Server::start(vec![route("/image.png", Reply::status(404))]).await;
            let url = server.url("/image.png");
            let error = remote(&["127.0.0.1"], 50.0)
                .image_details(&url)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Client error '404 Not Found' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/404"
                )
            );
            assert_eq!(server.request_count(), 1);
        }
        "remote_fetch_fails_with_informational_status_text" => {
            let server = Server::start(vec![route("/image.png", Reply::status(101))]).await;
            let url = server.url("/image.png");
            let error = remote(&["127.0.0.1"], 50.0)
                .image_details(&url)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Informational response '101 Switching Protocols' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/101"
                )
            );
        }
        "remote_fetch_fails_with_server_error_text" => {
            let server = Server::start(vec![route("/image.png", Reply::status(503))]).await;
            let url = server.url("/image.png");
            let error = remote(&["127.0.0.1"], 50.0)
                .image_details(&url)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Server error '503 Service Unavailable' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/503"
                )
            );
        }
        "remote_fetch_fails_with_invalid_status_text" => {
            let server = Server::start(vec![route("/image.png", Reply::status(600))]).await;
            let url = server.url("/image.png");
            let error = remote(&["127.0.0.1"], 50.0)
                .image_details(&url)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Invalid status code '600 ' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/600"
                )
            );
        }
        "remote_fetch_fails_with_redirect_status_text" => {
            let server = Server::start(vec![route("/image.png", Reply::status(304))]).await;
            let url = server.url("/image.png");
            let error = remote_with(TRANSPORT_LIMITS, &[], 50.0, false)
                .image_details(&url)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Redirect response '304 Not Modified' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/304"
                )
            );
        }
        "remote_fetch_names_the_last_request_url" => {
            let redirect = Reply {
                status: 302,
                headers: vec![("Location".to_owned(), "/final".to_owned())],
                body: Vec::new(),
                truncate_to: None,
                drip: false,
            };
            let server = Server::start(vec![
                route("/image.png", redirect),
                route("/final", Reply::status(404)),
            ])
            .await;
            let error = remote(&["127.0.0.1"], 50.0)
                .image_details(&server.url("/image.png"))
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Client error '404 Not Found' for url '{}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/404",
                    server.url("/final")
                )
            );
            assert_eq!(server.request_count(), 2);
        }
        "remote_fetch_fails_a_body_over_the_limit" => {
            let server = Server::start(vec![route(
                "/image.png",
                Reply::image("image/png", &vec![0xAB_u8; 2000]),
            )])
            .await;
            let url = server.url("/image.png");
            let error = remote(&["127.0.0.1"], 0.001)
                .image_details(&url)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Error: Image size (0.00MB) exceeds maximum allowed size (0.001MB). url={url}"
                )
            );
            assert_eq!(server.request_count(), 1);
        }
        "remote_fetch_rejects_a_blocked_loopback" => {
            let server =
                Server::start(vec![route("/image.png", Reply::image("image/png", PNG))]).await;
            let url = server.url("/image.png");
            let error = remote(&[], 50.0).image_details(&url).await.unwrap_err();
            assert_eq!(
                error.to_string(),
                "URL targets a blocked address (127.0.0.1). If this is a legitimate internal service, add the host to `user_url_allowed_hosts` in litellm_settings."
            );
            assert_eq!(server.request_count(), 0);
        }
        "remote_fetch_fails_a_truncated_body" => {
            let reply = Reply {
                status: 200,
                headers: vec![("Content-Type".to_owned(), "image/png".to_owned())],
                body: vec![0xAB_u8; 100],
                truncate_to: Some(10),
                drip: false,
            };
            let server = Server::start(vec![route("/image.png", reply)]).await;
            let url = server.url("/image.png");
            let error = remote(&["127.0.0.1"], 50.0)
                .image_details(&url)
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                BedrockImageError::Body(BodyReadError::Read(_))
            ));
        }
        "remote_fetch_ends_a_dripping_body_at_the_deadline" => {
            let server = Server::start(vec![route(
                "/drip.png",
                Reply {
                    status: 200,
                    headers: Vec::new(),
                    body: vec![b'x'; 20],
                    truncate_to: None,
                    drip: true,
                },
            )])
            .await;
            let url = server.url("/drip.png");
            let limits = TransportLimits {
                connect_timeout: Duration::from_secs(5),
                read_timeout: Duration::from_millis(100),
                body_bytes: TransportLimits::DEFAULT.body_bytes,
                header_bytes: TransportLimits::DEFAULT.header_bytes,
            };
            // the request's own 100ms timeout fires first, so only the body-read mapping can produce the deadline
            let mut client = remote_with(limits, &["127.0.0.1"], 50.0, true);
            client.deadline = TRANSPORT_LIMITS.read_timeout;
            let outcome = timeout(Duration::from_secs(5), client.image_details(&url))
                .await
                .expect("a fetch without any timeout would never end");
            assert!(matches!(outcome.unwrap_err(), BedrockImageError::Deadline));
        }
        "remote_fetch_ends_a_dripping_body_at_the_total_deadline" => {
            let server = Server::start(vec![route(
                "/drip.png",
                Reply {
                    status: 200,
                    headers: Vec::new(),
                    body: vec![b'x'; 20],
                    truncate_to: None,
                    drip: true,
                },
            )])
            .await;
            let url = server.url("/drip.png");
            let limits = TransportLimits {
                connect_timeout: Duration::from_secs(5),
                read_timeout: Duration::from_secs(5),
                body_bytes: TransportLimits::DEFAULT.body_bytes,
                header_bytes: TransportLimits::DEFAULT.header_bytes,
            };
            // the client's 5-second read and request timeouts never fire, so only the 100ms total deadline ends the fetch
            let mut client = remote_with(limits, &["127.0.0.1"], 50.0, true);
            client.deadline = Duration::from_millis(100);
            let outcome = timeout(Duration::from_secs(5), client.image_details(&url))
                .await
                .expect("a fetch without a total deadline would never end");
            assert!(matches!(outcome.unwrap_err(), BedrockImageError::Deadline));
        }
        "process_dispatches_base64_before_a_url" => {
            let error = process_image("https://example.com/base64.png", None, &remote(&[], 50.0))
                .await
                .unwrap_err();
            assert!(matches!(error, BedrockImageError::DataUrlParts));
        }
        "process_rejects_an_unsupported_image_url" => {
            let error = process_image("s3://bucket/image.png", None, &remote(&[], 50.0))
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Unsupported image type. Expected either image url or base64 encoded string"
            );
        }
        "process_overrides_the_media_type_and_format" => {
            let block = process_image(
                "data:image/png;base64,QUJD",
                Some("image/jpeg"),
                &remote(&[], 50.0),
            )
            .await
            .unwrap();
            assert_eq!(
                serde_json::to_string(&block).unwrap(),
                r#"{"image":{"format":"jpeg","source":{"bytes":"QUJD"}}}"#
            );
        }
        "process_override_sets_the_mime_type" => {
            let block = process_image(
                "data:image/png;base64,QUJD",
                Some("application/pdf"),
                &remote(&[], 50.0),
            )
            .await
            .unwrap();
            assert_eq!(
                serde_json::to_string(&block).unwrap(),
                r#"{"document":{"format":"pdf","source":{"bytes":"QUJD"},"name":"DocumentPDFmessages_0a702913308f6ec6_pdf"}}"#
            );
        }
        "process_override_without_a_subtype_fails" => {
            let error = process_image(
                "data:image/png;base64,QUJD",
                Some("png"),
                &remote(&[], 50.0),
            )
            .await
            .unwrap_err();
            assert!(matches!(
                error,
                BedrockImageError::MissingFormatSubtype { .. }
            ));
        }
        "process_fetches_a_remote_image" => {
            let server =
                Server::start(vec![route("/image.png", Reply::image("image/png", PNG))]).await;
            let url = server.url("/image.png");
            let block = process_image(&url, None, &remote(&["127.0.0.1"], 50.0))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_string(&block).unwrap(),
                format!(
                    r#"{{"image":{{"format":"png","source":{{"bytes":"{}"}}}}}}"#,
                    BASE64.encode(PNG)
                )
            );
            assert_eq!(server.request_count(), 1);
        }
        "process_rejects_an_unsupported_format" => {
            let error = process_image("data:image/tiff;base64,AAAA", None, &remote(&[], 50.0))
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                BedrockImageError::UnsupportedImageFormat { .. }
            ));
        }
        "process_returns_a_document_block" => {
            let block = process_image("data:application/pdf;base64,QUJD", None, &remote(&[], 50.0))
                .await
                .unwrap();
            let ContentBlock::Document { document } = block else {
                panic!("a document media type answers with a document block");
            };
            assert_eq!(document.format, "pdf");
            assert_eq!(document.source.bytes, "QUJD");
        }
        "process_uses_the_fetched_content_type" => {
            let server = Server::start(vec![route(
                "/clip.mp4",
                Reply::image("video/mp4", b"mp4-body"),
            )])
            .await;
            let url = server.url("/clip.mp4");
            let block = process_image(&url, None, &remote(&["127.0.0.1"], 50.0))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_string(&block).unwrap(),
                format!(
                    r#"{{"video":{{"format":"mp4","source":{{"bytes":"{}"}}}}}}"#,
                    BASE64.encode(b"mp4-body")
                )
            );
        }
        other => panic!("unknown scenario {other}"),
    }
}
