#![cfg(test)]

use std::fs;
use std::io;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use crate::api::image_contracts::records::{ImageData, ImageHttpResponse, ImageParams};
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::OpenRouterError;

use super::decode::{OpenRouterImageReply, ReplyDecodeError, decode_openrouter_image_reply};
use super::shared::{
    authorization, get_error_class, map_quality_to_image_size, map_size_to_aspect_ratio,
    resolved_base, resolved_key,
};

mod edit;
mod generation;

const CREATED: i64 = 1_700_000_000;
const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\npng-body";
const JPEG: &[u8] = b"\xff\xd8\xffjpeg-body";
const GIF: &[u8] = b"GIF89a-gif-body";
const WEBP: &[u8] = b"RIFF\x00\x00\x00\x00WEBPwebp";
const HEIC: &[u8] = b"\x00\x00\x00\x18ftypheic";

fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn json(value: &str) -> Json {
    serde_json::from_str(value).expect("the value must be valid JSON")
}

fn params(value: &str) -> ImageParams {
    serde_json::from_str(value).expect("the params must be valid JSON")
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

fn raw(status: u16, headers: Headers, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn ok(body: &str) -> ImageHttpResponse {
    raw(200, Headers::new(), body)
}

fn decode(body: &str) -> Result<OpenRouterImageReply, ReplyDecodeError> {
    decode_openrouter_image_reply(body.as_bytes())
}

fn b64(encoded: &str) -> ImageData {
    ImageData {
        b64_json: Some(String::from(encoded)),
        revised_prompt: None,
        url: None,
    }
}

fn url_image(url: &str) -> ImageData {
    ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(String::from(url)),
    }
}

fn image_part(mime: &str, bytes: &[u8]) -> Json {
    json(&format!(
        r#"{{"type": "image_url", "image_url": {{"url": "data:{mime};base64,{}"}}}}"#,
        STANDARD.encode(bytes)
    ))
}

fn content_parts(body: &ImageParams) -> Vec<Json> {
    let Some(Json::List(messages)) = body.get("messages") else {
        panic!("messages is a list");
    };
    let Some(Json::Object(message)) = messages.first() else {
        panic!("the message is an object");
    };
    let Some(Json::List(parts)) = message.get("content") else {
        panic!("content is a list");
    };
    parts.clone()
}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

impl Seek for FailingReader {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Ok(0)
    }
}

#[rstest]
#[case::small_square("256x256", "1:1")]
#[case::medium_square("512x512", "1:1")]
#[case::square("1024x1024", "1:1")]
#[case::landscape("1536x1024", "3:2")]
#[case::wide("1792x1024", "16:9")]
#[case::portrait("1024x1536", "2:3")]
#[case::tall("1024x1792", "9:16")]
#[case::auto("auto", "1:1")]
fn sizes_map_to_the_documented_aspect_ratios(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(map_size_to_aspect_ratio(size), ratio);
}

#[rstest]
#[case::unknown("2048x2048")]
#[case::uppercase("1024X1024")]
#[case::empty("")]
#[case::padded(" 1024x1024")]
fn an_unknown_size_gets_the_square_ratio(#[case] size: &str) {
    assert_eq!(map_size_to_aspect_ratio(size), "1:1");
}

#[rstest]
#[case::low("low", Some("1K"))]
#[case::standard("standard", Some("1K"))]
#[case::auto("auto", Some("1K"))]
#[case::medium("medium", Some("2K"))]
#[case::high("high", Some("4K"))]
#[case::hd("hd", Some("4K"))]
#[case::unknown("ultra", None)]
#[case::empty("", None)]
fn qualities_map_to_the_documented_image_sizes(
    #[case] quality: &str,
    #[case] image_size: Option<&str>,
) {
    assert_eq!(map_quality_to_image_size(quality), image_size);
}

#[test]
fn the_key_chain_skips_an_absent_or_empty_key() {
    let first = SecretString::from("first");
    let second = SecretString::from("second");
    let environment = SecretString::from("environment");
    let empty = SecretString::from("");
    let none = resolved_key([None, None, None]);
    assert!(none.is_none());
    let all_empty = resolved_key([Some(&empty), Some(&empty), Some(&empty)]);
    assert!(all_empty.is_none());
    let chosen = resolved_key([Some(&empty), Some(&second), Some(&environment)]);
    assert_eq!(
        chosen.map(|key| key.expose_secret().as_str()),
        Some("second")
    );
    let chosen = resolved_key([None, Some(&empty), Some(&environment)]);
    assert_eq!(
        chosen.map(|key| key.expose_secret().as_str()),
        Some("environment")
    );
    let chosen = resolved_key([Some(&first), Some(&second), Some(&environment)]);
    assert_eq!(
        chosen.map(|key| key.expose_secret().as_str()),
        Some("first")
    );
}

#[test]
fn the_base_chain_skips_an_absent_or_empty_base() {
    assert_eq!(resolved_base([None, None]), None);
    assert_eq!(resolved_base([Some(""), Some("")]), None);
    assert_eq!(
        resolved_base([Some(""), Some("environment")]),
        Some("environment")
    );
    assert_eq!(
        resolved_base([Some("api"), Some("environment")]),
        Some("api")
    );
}

#[test]
fn the_authorization_header_carries_the_key_or_none() {
    let key = SecretString::from("sk-or-key");
    assert_eq!(authorization(None), "Bearer None");
    assert_eq!(authorization(Some(&key)), "Bearer sk-or-key");
}

#[test]
fn the_error_class_of_both_configs_passes_the_message_status_and_headers_through() {
    let headers = headers(&[("X-Raw", "1")]);
    let error = get_error_class("failed", 429, headers.clone());
    assert_eq!(
        error,
        OpenRouterError {
            message: String::from("failed"),
            status_code: 429,
            headers,
        }
    );
}

#[test]
fn a_data_url_becomes_base64_with_no_url() {
    let reply = decode(
        r#"{"choices": [{"message": {"images": [{"image_url": {"url": "data:image/png;base64,AAAA"}}]}}]}"#,
    )
    .expect("the reply decodes");
    assert_eq!(reply.images, vec![b64("AAAA")]);
}

#[test]
fn a_data_url_without_a_comma_keeps_both_fields_empty() {
    let reply =
        decode(r#"{"choices": [{"message": {"images": [{"image_url": {"url": "data:"}}]}}]}"#)
            .expect("the reply decodes");
    assert_eq!(reply.images, vec![ImageData::default()]);
}

#[test]
fn a_plain_url_becomes_the_url() {
    let reply = decode(
        r#"{"choices": [{"message": {"images": [{"image_url": {"url": "https://example.test/a.png"}}]}}]}"#,
    )
    .expect("the reply decodes");
    assert_eq!(reply.images, vec![url_image("https://example.test/a.png")]);
}

#[test]
fn images_are_read_from_every_choice_and_unusable_entries_are_dropped() {
    let reply = decode(
        r#"{"choices": [{"message": {"images": [{}, {"image_url": {}}, {"image_url": {"url": null}}, {"image_url": {"url": ""}}, {"image_url": {"url": "data:image/webp;base64,BBBB"}}]}}, {"message": {"images": [{"image_url": {"url": "https://example.test/c.png"}}]}}]}"#,
    )
    .expect("the reply decodes");
    assert_eq!(
        reply.images,
        vec![b64("BBBB"), url_image("https://example.test/c.png")]
    );
}

#[test]
fn a_reply_without_usable_images_has_no_images() {
    let cases = [
        "{}",
        r#"{"choices": []}"#,
        r#"{"choices": [{}]}"#,
        r#"{"choices": [{"message": {}}]}"#,
        r#"{"choices": [{"message": {"images": []}}]}"#,
    ];
    let images: Vec<Vec<ImageData>> = cases
        .iter()
        .map(|body| decode(body).expect("the reply decodes").images)
        .collect();
    assert_eq!(images, vec![Vec::new(); cases.len()]);
}

#[test]
fn usage_is_read_when_it_is_a_non_empty_object() {
    let reply = decode(
        r#"{"usage": {"prompt_tokens": 7, "total_tokens": 9, "completion_tokens_details": {"image_tokens": 2}, "prompt_tokens_details": {"image_tokens": 1}, "cost": 0.25, "cost_details": {"upstream": 0.2}}}"#,
    )
    .expect("the reply decodes");
    let usage = reply.usage.expect("the usage is kept");
    assert_eq!(usage.prompt_tokens, Some(number(7)));
    assert_eq!(usage.total_tokens, Some(number(9)));
    assert_eq!(
        usage.completion_tokens_details,
        Some(json(r#"{"image_tokens": 2}"#))
    );
    assert_eq!(
        usage.prompt_tokens_details,
        Some(json(r#"{"image_tokens": 1}"#))
    );
    assert_eq!(usage.cost, Some(json("0.25")));
    assert_eq!(usage.cost_details, Some(json(r#"{"upstream": 0.2}"#)));
}

#[test]
fn an_absent_null_or_empty_usage_is_none() {
    let bodies = ["{}", r#"{"usage": null}"#, r#"{"usage": {}}"#];
    let usages: Vec<bool> = bodies
        .iter()
        .map(|body| decode(body).expect("the reply decodes").usage.is_none())
        .collect();
    assert_eq!(usages, vec![true; bodies.len()]);
}

#[test]
fn the_model_is_kept_as_the_reply_sends_it() {
    let model = decode(r#"{"model": "openrouter/model"}"#)
        .expect("the reply decodes")
        .model;
    assert_eq!(model, Some(text("openrouter/model")));
    let model = decode(r#"{"model": null}"#)
        .expect("the reply decodes")
        .model;
    assert_eq!(model, Some(Json::Null));
    let model = decode(r#"{"model": 5}"#).expect("the reply decodes").model;
    assert_eq!(model, Some(number(5)));
    let model = decode("{}").expect("the reply decodes").model;
    assert_eq!(model, None);
}

#[test]
fn a_body_that_is_not_json_is_a_parse_error() {
    let error =
        decode_openrouter_image_reply(b"<html>not json</html>").expect_err("the body is not JSON");
    assert!(matches!(error, ReplyDecodeError::Json { .. }));
    let empty = decode_openrouter_image_reply(b"").expect_err("the body is empty");
    assert!(matches!(empty, ReplyDecodeError::Json { .. }));
}

#[rstest]
#[case::nan(r#"{"usage": {"cost": NaN}}"#)]
#[case::infinity(r#"{"usage": {"cost": Infinity}}"#)]
#[case::overflowing_exponent(r#"{"usage": {"cost": 1e400}}"#)]
fn non_standard_json_is_a_parse_error(#[case] body: &str) {
    let error = decode_openrouter_image_reply(body.as_bytes()).expect_err("the body is not JSON");
    assert!(matches!(error, ReplyDecodeError::Json { .. }));
}

#[rstest]
#[case::body_list("[]")]
#[case::body_null("null")]
#[case::body_number("5")]
#[case::choices_object(r#"{"choices": {}}"#)]
#[case::choices_null(r#"{"choices": null}"#)]
#[case::choice_number(r#"{"choices": [5]}"#)]
#[case::message_null(r#"{"choices": [{"message": null}]}"#)]
#[case::images_object(r#"{"choices": [{"message": {"images": {}}}]}"#)]
#[case::entry_number(r#"{"choices": [{"message": {"images": [5]}}]}"#)]
#[case::image_url_number(r#"{"choices": [{"message": {"images": [{"image_url": 5}]}}]}"#)]
#[case::image_url_null(r#"{"choices": [{"message": {"images": [{"image_url": null}]}}]}"#)]
#[case::url_number(r#"{"choices": [{"message": {"images": [{"image_url": {"url": 5}}]}}]}"#)]
#[case::usage_list(r#"{"usage": []}"#)]
#[case::usage_number(r#"{"usage": 5}"#)]
fn a_wrong_shape_is_a_decode_error(#[case] body: &str) {
    let error = decode(body).expect_err("the shape is wrong");
    assert!(matches!(error, ReplyDecodeError::Shape(..)));
}

#[rstest]
#[case::choices(r#"{"choices": {}}"#, "choices must be a list")]
#[case::choice(r#"{"choices": [5]}"#, "each choice must be an object")]
#[case::message(r#"{"choices": [{"message": 5}]}"#, "message must be an object")]
#[case::images(
    r#"{"choices": [{"message": {"images": 5}}]}"#,
    "images must be a list"
)]
#[case::entry(
    r#"{"choices": [{"message": {"images": [5]}}]}"#,
    "each image must be an object"
)]
#[case::image_url(
    r#"{"choices": [{"message": {"images": [{"image_url": 5}]}}]}"#,
    "image_url must be an object"
)]
#[case::url(
    r#"{"choices": [{"message": {"images": [{"image_url": {"url": 5}}]}}]}"#,
    "image_url.url must be a string"
)]
#[case::usage(r#"{"usage": 5}"#, "usage must be an object")]
fn a_wrong_shape_names_the_field(#[case] body: &str, #[case] message: &str) {
    let error = decode(body).expect_err("the shape is wrong");
    assert_eq!(error.to_string(), message);
}

#[test]
fn replays_the_openrouter_image_reply_corpus() {
    let corpus =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openrouter_image_reply");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no OpenRouter image reply corpus seeds");
    replay_openrouter_image_replies(&paths);
}

fn replay_openrouter_image_replies(paths: &[PathBuf]) {
    for path in paths {
        let bytes = fs::read(path).expect("the corpus seed reads");
        drop(decode_openrouter_image_reply(&bytes));
    }
}
