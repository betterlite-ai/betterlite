#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use self::support::{
    BOUNDARY, FakeTransport, NOW, OK_BODY, Recorder, empty_request, failure_of, image,
    logged_records, number, object, options, prepared, request, run, send_and_record, supplied,
};
use crate::api::image_contracts::records::{ImageFile, ImageHeaders};
use crate::image_generation::decode::{decode_image_reply, original_response};
use crate::image_generation::reply::{ImageGenerationReply, ImageUsage, ImageUsageDetails};
use crate::image_variations::call::{DEFAULT_TIMEOUT, prepare_image_variation};
use crate::image_variations::client::normalize_base;
use crate::image_variations::error::{ImageVariationSource, OpenAiImageVariationError};
use crate::image_variations::logging::data_view;
use crate::image_variations::request::{
    begin_image_variation, finish_image_variation, send_image_variation,
};
use crate::image_variations::transform::usage_identity;
use crate::image_variations::wire::outbound_request;
use crate::image_variations::{
    ImageVariationData, ImageVariationParams, OpenAiImageVariationConfig,
};
use betterlite_http::client_kind::ClientKind;

#[rstest]
#[case::variation_image_part(0)]
#[case::n_size_format_user(1)]
#[case::omitted_default_fields(2)]
#[case::provider_error(3)]
#[case::binary_file_descriptor(4)]
#[case::variation_missing_config_data(5)]
#[case::variation_reply_identity(6)]
#[tokio::test]
async fn image_variations_cases(#[case] case: u8) {
    match case {
        0 => variation_image_part().await,
        1 => n_size_format_user(),
        2 => omitted_default_fields().await,
        3 => provider_error().await,
        4 => binary_file_descriptor().await,
        5 => variation_missing_config_data().await,
        6 => variation_reply_identity().await,
        _ => panic!("unknown case {case}"),
    }
}

fn transform_part() {
    let supplied_image = image(Some("cat.png"), "PNGDATA");
    let data = OpenAiImageVariationConfig::request_data(
        supplied_image.clone(),
        options(r#"{"n":2,"size":"512x512"}"#),
    )
    .unwrap();
    assert_eq!(data.image, supplied_image);
    assert_eq!(data.options.keys().collect::<Vec<_>>(), vec!["n", "size"]);
    assert!(!data.options.contains_key("model"));
    let bare =
        OpenAiImageVariationConfig::request_data(supplied_image.clone(), IndexMap::new()).unwrap();
    assert!(bare.options.is_empty());
    // an option named image cannot replace the uploaded file, so the transform yields no data
    let rejected = OpenAiImageVariationConfig::request_data(
        supplied_image,
        options(r#"{"image":"https://example.test/cat.png"}"#),
    );
    assert!(rejected.is_none());
}

async fn variation_image_part() {
    transform_part();
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let mut call = request(image(Some("cat.png"), "PNGDATA"));
    call.optional_params = options(r#"{"n":2,"size":"512x512"}"#);
    let prepared = begin_image_variation(call, &mut recorder).unwrap();
    assert_eq!(prepared.data.options.get("n"), Some(&number(2)));
    let sent = send_image_variation(&prepared, &transport).await.unwrap();
    finish_image_variation(&prepared, Ok(sent), &mut recorder).unwrap();
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].names(), vec!["n", "size", "image"]);
    assert_eq!(calls[0].part("n").unwrap().data, b"2");
    assert_eq!(calls[0].part("size").unwrap().data, b"512x512");
    let part = calls[0].part("image").unwrap();
    assert_eq!(part.data, b"PNGDATA");
    assert_eq!(part.file_name.as_deref(), Some("cat.png"));
    assert_eq!(part.content_type.as_deref(), Some("image/png"));
    assert_eq!(calls[0].url, "https://example.test/v1/images/variations");
    assert_eq!(recorder.pre.len(), 1);
    assert_eq!(recorder.post.len(), 1);
}

fn n_size_format_user() {
    assert_eq!(
        OpenAiImageVariationConfig::SUPPORTED_PARAMS,
        ["n", "size", "response_format", "user"]
    );
    let mut existing = options(r#"{"n":1,"size":"256x256"}"#);
    let non_default = options(
        r#"{"n":2,"size":"512x512","response_format":"b64_json","user":"u1","style":"vivid"}"#,
    );
    OpenAiImageVariationConfig::map_params(&non_default, &mut existing);
    assert_eq!(existing.get("n"), Some(&number(2)));
    assert_eq!(
        existing.get("size"),
        Some(&Json::Text(String::from("512x512")))
    );
    assert_eq!(
        existing.get("response_format"),
        Some(&Json::Text(String::from("b64_json")))
    );
    assert_eq!(existing.get("user"), Some(&Json::Text(String::from("u1"))));
    // map_openai_params copies every entry, so an unsupported parameter survives the mapping
    assert_eq!(
        existing.get("style"),
        Some(&Json::Text(String::from("vivid")))
    );
    let mut untouched = options(r#"{"size":"1024x1024"}"#);
    OpenAiImageVariationConfig::map_params(&IndexMap::new(), &mut untouched);
    assert_eq!(
        untouched.get("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    let mut kept = options(r#"{"max_retries":5}"#);
    OpenAiImageVariationConfig::map_params(&options(r#"{"max_retries":7}"#), &mut kept);
    assert_eq!(kept.get("max_retries"), Some(&number(7)));
}

async fn supplied_client_fields() {
    let client = supplied(ClientKind::Sync);
    let mut call = request(image(Some("cat.png"), "PNGDATA"));
    call.client = Some(client.clone());
    call.api_key = None;
    let prepared = prepared(call).unwrap();
    assert_eq!(prepared.client.kind, ClientKind::Sync);
    assert_eq!(prepared.client.base_url, client.base_url);
    assert_eq!(prepared.client.max_retries, 1);
    assert_eq!(prepared.client.timeout, Some(Duration::from_secs(45)));
    assert!(!prepared.client.shared_session);
    assert_eq!(
        prepared.client.organization.as_deref(),
        Some("org-supplied")
    );
    let outgoing = send_and_record(&prepared).await;
    assert_eq!(
        outgoing.header("authorization").as_deref(),
        Some("Bearer sk-supplied")
    );
}

async fn built_client_fields() {
    let mut call = request(image(None, "PNGDATA"));
    call.optional_params = options(r#"{"max_retries":5}"#);
    call.organization = Some(String::from("org-test"));
    let prepared = prepared(call).unwrap();
    assert_eq!(prepared.client.kind, ClientKind::Sync);
    assert_eq!(prepared.client.base_url, "https://example.test/v1/");
    assert_eq!(prepared.client.max_retries, 5);
    assert_eq!(prepared.client.timeout, Some(Duration::from_secs(30)));
    assert!(prepared.client.shared_session);
    assert_eq!(prepared.client.organization.as_deref(), Some("org-test"));
    assert!(!prepared.data.options.contains_key("max_retries"));
    let outgoing = send_and_record(&prepared).await;
    assert_eq!(outgoing.deadline, Duration::from_secs(30));
    assert_eq!(
        outgoing.header("openai-organization").as_deref(),
        Some("org-test")
    );
    let part = outgoing.part("image").unwrap();
    assert_eq!(part.file_name.as_deref(), Some("upload"));
    assert_eq!(
        part.content_type.as_deref(),
        Some("application/octet-stream")
    );
}

async fn omitted_default_fields() {
    supplied_client_fields().await;
    built_client_fields().await;
    let mut defaulted = request(image(None, "PNGDATA"));
    defaulted.timeout = None;
    let defaulted_prepared = prepared(defaulted).unwrap();
    assert_eq!(defaulted_prepared.client.max_retries, 2);
    assert_eq!(defaulted_prepared.client.timeout, None);
    assert_eq!(defaulted_prepared.deadline, DEFAULT_TIMEOUT);
    let mut delegated = request(image(None, "PNGDATA"));
    delegated.params = ImageVariationParams { async_call: true };
    let delegated_prepared = prepared(delegated).unwrap();
    assert_eq!(delegated_prepared.client.kind, ClientKind::Asynchronous);
    assert!(delegated_prepared.async_mode);
    let sync = request(image(None, "PNGDATA"));
    assert!(!prepared(sync).unwrap().async_mode);
    let mut unkeyed = empty_request();
    unkeyed.image = image(None, "PNGDATA");
    unkeyed.config = Some(OpenAiImageVariationConfig);
    let failure = prepare_image_variation(unkeyed).unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(failure.message.contains("API key"));
}

async fn provider_error() {
    let status = FakeTransport::answering(
        429,
        &[("x-request-id", "req-429")],
        r#"{"error":{"message":"slow down"}}"#,
    );
    let mut recorder = Recorder::default();
    let failure = failure_of(request(image(None, "PNGDATA")), &status, &mut recorder).await;
    assert_eq!(failure.status_code, 429);
    assert_eq!(
        failure.headers,
        Some(ImageHeaders::from([(
            String::from("x-request-id"),
            String::from("req-429"),
        )]))
    );
    assert!(failure.message.contains("slow down"));
    // the failed call still ran pre_call, and never ran post_call
    assert_eq!(recorder.pre.len(), 1);
    assert!(recorder.post.is_empty());
}

#[tokio::test]
async fn a_non_utf8_error_body_keeps_its_status_and_headers() {
    let transport = FakeTransport::answering_bytes(
        429,
        &[("x-request-id", "req-bytes")],
        b"bad \xFF body".to_vec(),
    );
    let mut recorder = Recorder::default();
    let failure = failure_of(request(image(None, "PNGDATA")), &transport, &mut recorder).await;
    assert_eq!(failure.status_code, 429);
    assert_eq!(
        failure.headers,
        Some(ImageHeaders::from([(
            String::from("x-request-id"),
            String::from("req-bytes"),
        )]))
    );
    assert_eq!(failure.message, "bad \u{FFFD} body");
    assert_eq!(recorder.pre.len(), 1);
    assert!(recorder.post.is_empty());
}

#[tokio::test]
async fn transport_and_reply_errors_are_500s() {
    let mut recorder = Recorder::default();
    let failure = failure_of(
        request(image(None, "PNGDATA")),
        &FakeTransport::failing(),
        &mut recorder,
    )
    .await;
    assert_eq!(failure.status_code, 500);
    assert_eq!(failure.headers, None);
    assert!(matches!(
        *failure.source,
        ImageVariationSource::Transport(_)
    ));
    let mut delegated = request(image(None, "PNGDATA"));
    delegated.params = ImageVariationParams { async_call: true };
    let delegated_failure = failure_of(delegated, &FakeTransport::failing(), &mut recorder).await;
    assert_eq!(delegated_failure.status_code, 500);
    // the SDK parses the reply as ImagesResponse, so an error object is a 500, not a 422
    let error_object = FakeTransport::answering(200, &[], r#"{"error":{"message":"boom"}}"#);
    let reply_failure = failure_of(
        request(image(None, "PNGDATA")),
        &error_object,
        &mut recorder,
    )
    .await;
    assert_eq!(reply_failure.status_code, 500);
    assert!(matches!(
        *reply_failure.source,
        ImageVariationSource::Reply(_)
    ));
}

#[tokio::test]
async fn an_unsupported_option_is_the_sdk_keyword_error() {
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let mut call = request(image(Some("cat.png"), "PNGDATA"));
    call.optional_params = options(r#"{"style":"vivid"}"#);
    let failure = failure_of(call, &transport, &mut recorder).await;
    assert_eq!(failure.status_code, 500);
    assert_eq!(failure.headers, None);
    assert!(matches!(
        *failure.source,
        ImageVariationSource::Unsupported { .. }
    ));
    assert!(transport.calls().is_empty());
    assert_eq!(recorder.pre.len(), 1);
    assert!(recorder.post.is_empty());
}

#[tokio::test]
async fn a_non_text_extra_header_names_its_key() {
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let mut call = request(image(None, "PNGDATA"));
    call.optional_params = options(r#"{"extra_headers":{"x-count":2}}"#);
    let failure = failure_of(call, &transport, &mut recorder).await;
    assert_eq!(failure.status_code, 500);
    assert!(failure.message.contains("x-count"));
    assert!(matches!(
        &*failure.source,
        ImageVariationSource::Option { field } if field == "x-count"
    ));
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn sdk_multipart_and_query_encoding() {
    let mut call = request(image(Some("cat.ico"), "ICODATA"));
    call.optional_params = options(
        r#"{"n":2,"extra_query":{"api-version":"1","drop":"","gone":null,"tags":["a","b"],"flag":true},"extra_body":{"n":3,"nested":{"a":1},"entries":[{"k":1},{"k":2}],"drop":"","gone":null}}"#,
    );
    let prepared = prepare_image_variation(call).unwrap();
    let outgoing = send_and_record(&prepared).await;
    assert_eq!(
        outgoing.query,
        vec![
            (String::from("api-version"), String::from("1")),
            (String::from("tags[]"), String::from("a")),
            (String::from("tags[]"), String::from("b")),
            (String::from("flag"), String::from("true")),
        ]
    );
    assert_eq!(
        outgoing.names(),
        vec!["n", "nested[a]", "entries[][k]", "entries[][k]", "image"]
    );
    assert_eq!(
        outgoing.values(),
        vec![
            b"3".as_slice(),
            b"1".as_slice(),
            b"1".as_slice(),
            b"2".as_slice(),
            b"ICODATA".as_slice(),
        ]
    );
    let part = outgoing.part("image").unwrap();
    assert_eq!(part.file_name.as_deref(), Some("cat.ico"));
    assert_eq!(
        part.content_type.as_deref(),
        Some("image/vnd.microsoft.icon")
    );
}

#[tokio::test]
async fn the_timeout_option_overrides_the_client_deadline() {
    let mut overridden = request(image(None, "PNGDATA"));
    overridden.optional_params = options(r#"{"timeout":5}"#);
    let prepared = prepare_image_variation(overridden).unwrap();
    let outgoing = send_and_record(&prepared).await;
    assert_eq!(outgoing.deadline, Duration::from_secs(5));
    let mut kept = request(image(None, "PNGDATA"));
    kept.optional_params = options(r#"{"timeout":null}"#);
    let kept_prepared = prepare_image_variation(kept).unwrap();
    assert_eq!(
        send_and_record(&kept_prepared).await.deadline,
        Duration::from_secs(30)
    );
    let mut rejected = request(image(None, "PNGDATA"));
    rejected.optional_params = options(r#"{"timeout":"soon"}"#);
    let rejected_prepared = prepare_image_variation(rejected).unwrap();
    let Err(source) = outbound_request(&rejected_prepared) else {
        panic!("a text timeout is not an SDK timeout");
    };
    assert!(matches!(
        &source,
        ImageVariationSource::Option { field } if field == "timeout"
    ));
}

#[test]
fn normalize_base_keeps_one_trailing_slash() {
    assert_eq!(
        normalize_base("https://example.test/v1").unwrap(),
        "https://example.test/v1/"
    );
    assert_eq!(
        normalize_base("https://example.test/v1/").unwrap(),
        "https://example.test/v1/"
    );
}

#[tokio::test]
async fn an_empty_upload_name_sends_no_content_type() {
    let prepared = prepare_image_variation(request(image(Some(""), "EMPTYNAME"))).unwrap();
    let outgoing = send_and_record(&prepared).await;
    let part = outgoing.part("image").unwrap();
    assert_eq!(part.file_name, None);
    assert_eq!(part.content_type, None);
    assert_eq!(part.data, b"EMPTYNAME");
}

async fn binary_file_descriptor() {
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let outcome = run(
        request(image(Some("cat.png"), "PNGDATA")),
        &transport,
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(outcome.reply.created, 7);
    let calls = transport.calls();
    let part = calls[0].part("image").unwrap();
    assert_eq!(part.name, "image");
    assert_eq!(part.data, b"PNGDATA");
    assert_eq!(part.file_name.as_deref(), Some("cat.png"));
    assert_eq!(part.content_type.as_deref(), Some("image/png"));
    assert_eq!(recorder.post.len(), 1);
    nested_upload_names().await;
    tuple_upload_name().await;
}

// httpx names a file object's part after Path(value.name).name, the last component
async fn nested_upload_names() {
    let nested_prepared = prepared(request(image(Some("dir/cat.png"), "NESTEDDATA"))).unwrap();
    let nested = send_and_record(&nested_prepared).await;
    let nested_part = nested.part("image").unwrap();
    assert_eq!(nested_part.file_name.as_deref(), Some("cat.png"));
    assert_eq!(nested_part.content_type.as_deref(), Some("image/png"));
    // a trailing slash is no path component either
    let slashed_prepared = prepared(request(image(Some("dir/cat.png/"), "SLASHDATA"))).unwrap();
    let slashed = send_and_record(&slashed_prepared).await;
    assert_eq!(
        slashed.part("image").unwrap().file_name.as_deref(),
        Some("cat.png")
    );
}

// a tuple filename keeps its value as given, beside the tuple's own content type
async fn tuple_upload_name() {
    let typed = ImageFile {
        filename: Some(String::from("dir/pic.webp")),
        content: b"WEBP".to_vec(),
        content_type: Some(String::from("image/webp")),
    };
    let typed_prepared = prepared(request(typed)).unwrap();
    let sent = send_and_record(&typed_prepared).await;
    let part = sent.part("image").unwrap();
    assert_eq!(part.file_name.as_deref(), Some("dir/pic.webp"));
    assert_eq!(part.content_type.as_deref(), Some("image/webp"));
    assert_eq!(part.data, b"WEBP");
}

async fn variation_missing_config_data() {
    let mut missing = request(image(None, "PNGDATA"));
    missing.config = None;
    let config_failure = prepare_image_variation(missing).unwrap_err();
    assert_eq!(config_failure.status_code, 500);
    assert!(
        config_failure
            .message
            .contains("provider not found: openai")
    );
    assert!(matches!(
        *config_failure.source,
        ImageVariationSource::MissingConfig { provider: "openai" }
    ));
    let mut false_data = request(image(None, "PNGDATA"));
    false_data.optional_params = options(r#"{"image":"not-a-file"}"#);
    let data_failure = prepare_image_variation(false_data).unwrap_err();
    assert_eq!(data_failure.status_code, 500);
    assert!(data_failure.message.contains("data field is required"));
    assert!(matches!(
        *data_failure.source,
        ImageVariationSource::MissingData
    ));
    // neither seam reaches the network or the logging hooks
    let transport = FakeTransport::ok();
    let mut recorder = Recorder::default();
    let mut unconfigured = request(image(None, "PNGDATA"));
    unconfigured.config = None;
    let failure = run(unconfigured, &transport, &mut recorder)
        .await
        .unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(transport.calls().is_empty());
    assert!(recorder.pre.is_empty());
}

async fn variation_reply_identity() {
    plain_reply_identity().await;
    rich_reply_identity().await;
}

async fn plain_reply_identity() {
    let body = r#"{"created":7,"data":[{"b64_json":"YQ=="}],"quality":"medium","size":"1024x1024","usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":1,"text_tokens":3},"output_tokens":8,"total_tokens":12}}"#;
    let transport = FakeTransport::answering(200, &[("x-request-id", "req-1")], body);
    let mut recorder = Recorder::default();
    let outcome = run(request(image(None, "PNGDATA")), &transport, &mut recorder)
        .await
        .unwrap();
    reply_fields(&outcome.reply);
    let dumped = serde_json::to_string(&outcome.reply).unwrap();
    assert!(!dumped.contains("\"prompt_tokens\""));
    assert!(!dumped.contains("\"prompt_tokens_details\""));
    assert!(dumped.contains(r#""output_tokens_details":null"#));
    let unchanged = decode_image_reply(body.as_bytes(), NOW).unwrap();
    assert_eq!(outcome.reply.data, unchanged.data);
    let logged = object(&recorder.post[0]);
    let original = object(logged.get("original_response").unwrap());
    assert_eq!(original.get("created"), Some(&number(7)));
    let Some(Json::List(items)) = original.get("data") else {
        panic!("the logged reply carries data");
    };
    assert!(object(&items[0]).get("provider_specific_fields").is_none());
    assert_eq!(recorder.pre.len(), 1);
    assert_eq!(recorder.post.len(), 1);
    let bodyless = FakeTransport::answering(200, &[], r#"{"data":[]}"#);
    let bodyless_outcome = run(request(image(None, "PNGDATA")), &bodyless, &mut recorder)
        .await
        .unwrap();
    assert_eq!(bodyless_outcome.reply.created, NOW);
    assert!(bodyless_outcome.reply.data.is_empty());
}

async fn rich_reply_identity() {
    let body = r#"{"created":7,"data":[],"usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":1,"text_tokens":3},"output_tokens":8,"output_tokens_details":{"image_tokens":6,"text_tokens":2,"reasoning_tokens":1},"total_tokens":12,"vendor":{"kind":"x"},"prompt_tokens":4,"completion_tokens":8}}"#;
    let transport = FakeTransport::answering(200, &[], body);
    let mut recorder = Recorder::default();
    let outcome = run(request(image(None, "PNGDATA")), &transport, &mut recorder)
        .await
        .unwrap();
    let usage = &outcome.reply.usage;
    assert_eq!(usage.input_tokens, 4);
    assert_eq!(usage.total_tokens, 12);
    assert_eq!(usage.prompt_tokens, Some(4));
    assert_eq!(usage.completion_tokens, Some(8));
    assert_eq!(
        object(usage.extra.get("vendor").unwrap()).get("kind"),
        Some(&Json::Text(String::from("x")))
    );
    let details = object(usage.extra.get("output_tokens_details").unwrap());
    assert_eq!(details.get("reasoning_tokens"), Some(&number(1)));
    assert_eq!(details.get("image_tokens"), Some(&number(6)));
    // the decoder's key order wrappers are not part of the wire reply
    assert!(usage.extra.get("prompt_tokens_details").is_none());
    assert!(usage.extra.get("completion_tokens_details").is_none());
    assert!(usage.extra.get("prompt_tokens").is_none());
    let dumped = serde_json::to_string(&outcome.reply).unwrap();
    assert!(dumped.contains(r#""reasoning_tokens":1"#));
    assert!(dumped.contains(r#""vendor":{"kind":"x"}"#));
    assert_eq!(recorder.post.len(), 1);
}

fn reply_fields(reply: &ImageGenerationReply) {
    assert_eq!(reply.created, 7);
    assert_eq!(reply.data[0].b64_json.as_deref(), Some("YQ=="));
    assert_eq!(reply.quality.as_deref(), Some("medium"));
    assert_eq!(reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(reply.usage.input_tokens, 4);
    assert_eq!(reply.usage.total_tokens, 12);
    assert_eq!(reply.usage.prompt_tokens, None);
    assert_eq!(reply.usage.completion_tokens, None);
    assert_eq!(
        reply.usage.extra.get("output_tokens_details"),
        Some(&Json::Null)
    );
    assert!(reply.usage.extra.get("prompt_tokens_details").is_none());
}

#[test]
fn image_variations_logs_the_pre_call_then_the_post_call() {
    let mut call = request(image(Some("cat.png"), "PNGDATA"));
    call.headers = Some(ImageHeaders::from([(
        String::from("x-caller"),
        String::from("yes"),
    )]));
    let recorded = logged_records(call);
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        object(&recorded[0]).get("log_event_type"),
        Some(&Json::Text(String::from("pre_api_call")))
    );
    assert_eq!(
        object(&recorded[1]).get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let pre = object(&recorded[0]);
    assert_eq!(pre.get("input"), Some(&Json::Text(String::new())));
    let arguments = object(pre.get("additional_args").unwrap());
    // the logs record the caller's api_base as sent, not the SDK client's normalized one
    assert_eq!(
        arguments.get("api_base"),
        Some(&Json::Text(String::from("https://example.test/v1")))
    );
    let headers = object(arguments.get("headers").unwrap());
    assert!(headers.contains_key("x-caller"));
    let post = object(&recorded[1]);
    assert_eq!(post.get("input"), Some(&Json::Null));
    // post_call keeps a dict reply as its JSON text, the way LiteLLM's json.dumps does
    let Some(Json::Text(original)) = post.get("original_response") else {
        panic!("the post call stores the reply as JSON text");
    };
    let parsed: Json = serde_json::from_str(original).unwrap();
    assert_eq!(object(&parsed).get("created"), Some(&number(7)));
}

#[test]
fn the_pre_call_record_names_the_upload_instead_of_its_bytes() {
    let recorded = logged_records(request(image(Some("cat.png"), "PNGDATA")));
    let pre = object(&recorded[0]);
    let arguments = object(pre.get("additional_args").unwrap());
    let body = object(arguments.get("complete_input_dict").unwrap());
    let data = object(body.get("data").unwrap());
    assert!(data.get("size").is_none());
    let image_view = object(data.get("image").unwrap());
    assert_eq!(
        image_view.get("filename"),
        Some(&Json::Text(String::from("cat.png")))
    );
    assert_eq!(image_view.get("size"), Some(&number(7)));
}

#[test]
fn outbound_request_carries_the_sdk_extras() {
    let mut call = request(image(None, "PNGDATA"));
    call.optional_params = options(
        r#"{"n":2,"size":"","extra_headers":{"x-old":"no","x-test":"yes"},"extra_query":{"api-version":"1"},"extra_body":{"seed":7}}"#,
    );
    let prepared = prepare_image_variation(call).unwrap();
    let outbound = outbound_request(&prepared).unwrap();
    assert_eq!(
        outbound.url.as_str(),
        "https://example.test/v1/images/variations"
    );
    assert_eq!(
        outbound.query,
        vec![(String::from("api-version"), String::from("1"))]
    );
    assert_eq!(
        outbound
            .headers
            .get("x-test")
            .and_then(|value| value.to_str().ok()),
        Some("yes")
    );
    let RequestBody::Multipart { parts, boundary } = outbound.body else {
        panic!("the variation request is multipart");
    };
    assert_eq!(boundary, BOUNDARY);
    let names: Vec<_> = parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(names, vec!["n", "seed", "image"]);
    assert_eq!(parts[0].data, b"2");
    assert_eq!(parts[1].data, b"7");
    assert_eq!(parts[2].data, b"PNGDATA");
}

#[tokio::test]
async fn a_supplied_client_of_the_other_kind_is_refused() {
    let mut call = request(image(None, "PNGDATA"));
    call.client = Some(supplied(ClientKind::Asynchronous));
    let failure = prepare_image_variation(call).unwrap_err();
    assert_eq!(failure.status_code, 500);
    assert!(matches!(
        *failure.source,
        ImageVariationSource::ClientKind {
            supplied: "async",
            needed: "sync"
        }
    ));
}

#[test]
fn the_error_class_keeps_the_message_and_headers() {
    let error = OpenAiImageVariationError::wrapped(ImageVariationSource::Status {
        status: 400,
        message: String::from("bad request"),
        headers: Some(ImageHeaders::from([(String::from("x"), String::from("y"))])),
    });
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "bad request");
    assert_eq!(
        error.headers.unwrap().get("x").map(String::as_str),
        Some("y")
    );
    let wrapped = OpenAiImageVariationError::wrapped(ImageVariationSource::MaxRetries);
    assert_eq!(wrapped.status_code, 500);
    assert_eq!(wrapped.headers, None);
}

#[test]
fn the_identity_reply_keeps_the_sdk_shape() {
    let body = br#"{"created":7,"data":[]}"#;
    let reply = decode_image_reply(body, NOW).unwrap();
    let original = original_response(body).unwrap();
    let usage = usage_identity(reply.usage, &original);
    assert!(usage.extra.is_empty());
    let identity = ImageGenerationReply { usage, ..reply };
    assert_eq!(identity.created, 7);
    let dumped = serde_json::to_string(&identity).unwrap();
    assert!(dumped.contains(r#""background":null"#));
    assert!(dumped.contains(r#""input_tokens":0"#));
    assert!(!dumped.contains("output_tokens_details"));
    // a usage the decoder built by hand keeps only what the reply carried
    let priced = usage_identity(
        ImageUsage {
            input_tokens: 1,
            input_tokens_details: ImageUsageDetails::default(),
            output_tokens: 2,
            total_tokens: 3,
            prompt_tokens: Some(1),
            completion_tokens: Some(2),
            extra: IndexMap::from([(String::from("vendor"), Json::Bool(true))]),
        },
        &original,
    );
    let priced_dump = serde_json::to_string(&priced).unwrap();
    assert!(!priced_dump.contains("prompt_tokens"));
    assert!(!priced_dump.contains("completion_tokens"));
    assert!(!priced_dump.contains("vendor"));
    assert!(!priced_dump.contains("output_tokens_details"));
}

#[test]
fn image_variation_params_default_to_the_sync_call() {
    assert!(!ImageVariationParams::default().async_call);
    assert_eq!(ClientKind::Sync.label(), "sync");
    assert_eq!(ClientKind::Asynchronous.label(), "async");
    let data = ImageVariationData {
        image: image(Some("cat.png"), "PNGDATA"),
        options: options(r#"{"n":2}"#),
    };
    let view = data_view(&data);
    let inner = object(view.get("data").unwrap());
    assert_eq!(inner.get("n"), Some(&number(2)));
    assert_eq!(
        object(inner.get("image").unwrap()).get("size"),
        Some(&number(7))
    );
}

async fn serve_variation_reply(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !received.windows(8).any(|window| window == b"PNGDATA\r") {
        let read = stream.read(&mut chunk).await.unwrap();
        received.extend_from_slice(&chunk[..read]);
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-request-id: req-9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{OK_BODY}",
        OK_BODY.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(received).unwrap()
}

#[tokio::test]
async fn proxied_client_posts_multipart_to_the_variations_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut local = request(image(Some("cat.png"), "PNGDATA"));
    local.api_base = Some(format!("http://{address}/v1"));
    local.boundary = String::from("wire-boundary");
    let prepared = prepare_image_variation(local).unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let (request_head, sent) = tokio::join!(
        serve_variation_reply(listener),
        send_image_variation(&prepared, &client)
    );
    let outcome = sent.unwrap();
    let lowered_head = request_head.to_lowercase();
    assert!(
        lowered_head.starts_with("post /v1/images/variations http/1.1"),
        "{lowered_head}"
    );
    assert!(
        lowered_head.contains("authorization: bearer sk-test"),
        "{lowered_head}"
    );
    assert!(
        lowered_head.contains("content-type: multipart/form-data; boundary=wire-boundary"),
        "{lowered_head}"
    );
    assert!(
        lowered_head.contains("name=\"image\"; filename=\"cat.png\""),
        "{lowered_head}"
    );
    assert!(lowered_head.contains("pngdata"), "{lowered_head}");
    assert_eq!(outcome.reply.created, 7);
}
