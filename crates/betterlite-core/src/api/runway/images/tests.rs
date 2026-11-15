#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use http::HeaderMap;
use http::header::AUTHORIZATION;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use url::Url;

use super::RunwayImageConfig;
use super::SUPPORTED_OPENAI_PARAMS;
use super::cost::{CUSTOM_LLM_PROVIDER, cost_calculator};
use super::error::RunwayImageError;
use super::params::{map_openai_params, request_body};
use super::reply::apply_task_output;
use super::transform::RunwayImageTransform;
use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;
use crate::api::runway::tasks::poll::{PollClock, TaskTransport};
use crate::api::runway::tasks::record::{RunwayTask, decode_runway_task};
use crate::api::runway::tasks::settings::RunwaySettings;
use crate::cost::image_dispatch::policy::{ImageResponseView, PricedImage};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn headers(pairs: &[(&str, &str)]) -> ImageHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(pairs: &[(&str, &str)]) -> RunwaySettings {
    let lookup: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RunwaySettings::load(&lookup).unwrap()
}

fn response(created: i64, data: Option<Vec<ImageData>>) -> ImageResponse {
    ImageResponse {
        created,
        data,
        usage: None,
    }
}

fn url_image(url: &str) -> ImageData {
    ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(url.to_owned()),
    }
}

fn task(body: &str) -> RunwayTask {
    decode_runway_task(body.as_bytes()).unwrap()
}

struct Script {
    answers: Mutex<VecDeque<(u16, &'static str)>>,
    requests: Mutex<Vec<(String, HeaderMap)>>,
}

impl Script {
    fn new(answers: &[(u16, &'static str)]) -> Self {
        Self {
            answers: Mutex::new(answers.iter().copied().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), headers));
        let (status, body) = self.answers.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }))
    }
}

struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<Duration>>,
}

impl Clock {
    fn new(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
            pauses: Mutex::new(Vec::new()),
        }
    }
}

impl PollClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses.lock().unwrap().push(duration);
        ready(())
    }
}

#[rstest]
#[case::known("gen4_image")]
#[case::provider_prefixed("runwayml/gen4_image")]
#[case::unknown("some-other-image-model")]
#[case::empty("")]
fn selector_answers_every_model(#[case] model: &str) {
    let lookup = BTreeMap::from([
        (
            "RUNWAYML_API_BASE".to_owned(),
            "https://other.test".to_owned(),
        ),
        (
            "RUNWAYML_DEFAULT_API_VERSION".to_owned(),
            "2025-01-01".to_owned(),
        ),
        ("RUNWAYML_POLLING_TIMEOUT".to_owned(), "30".to_owned()),
    ]);
    let config = RunwayImageConfig::for_model(model, &lookup).unwrap();
    assert_eq!(config.settings.api_base, "https://other.test");
    assert_eq!(config.settings.api_version, "2025-01-01");
    assert_eq!(config.settings.polling_timeout_seconds, 30);
}

#[test]
fn selector_needs_no_credentials() {
    let config = RunwayImageConfig::for_model("gen4_image", &BTreeMap::new()).unwrap();
    assert_eq!(config.settings.api_base, "https://api.dev.runwayml.com");
    assert_eq!(config.settings.api_version, "2024-11-06");
    assert_eq!(config.settings.polling_timeout_seconds, 600);
    assert!(config.settings.api_secret.is_none());
    assert!(config.settings.api_key.is_none());
}

#[test]
fn only_size_is_advertised() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["size"]);
}

#[rstest]
#[case::square("1024x1024", "1024:1024")]
#[case::landscape("1792x1024", "1792:1024")]
#[case::portrait("1024x1792", "1024:1792")]
#[case::wide("1920x1080", "1920:1080")]
#[case::tall("1080x1920", "1080:1920")]
#[case::unknown("800x600", "1920:1080")]
#[case::empty("", "1920:1080")]
#[case::case_sensitive("1024X1024", "1920:1080")]
fn presets_map_to_ratios_and_keep_the_size(#[case] size: &str, #[case] ratio: &str) {
    let input = params(&[("size", text(size))]);
    let mapped = map_openai_params(&input, ImageParams::new(), "gen4_image", false).unwrap();
    assert_eq!(mapped["ratio"], text(ratio));
    assert_eq!(mapped["size"], text(size));
    let body = request_body("gen4_image", "a cat", &mapped);
    assert_eq!(body["model"], text("gen4_image"));
    assert_eq!(body["promptText"], text("a cat"));
    assert_eq!(body["ratio"], text(ratio));
    assert!(!body.contains_key("size"));
}

#[test]
fn a_null_size_takes_the_default_ratio() {
    let input = params(&[("size", Json::Null)]);
    let mapped = map_openai_params(&input, ImageParams::new(), "gen4_image", false).unwrap();
    assert_eq!(mapped["ratio"], text("1920:1080"));
    assert_eq!(mapped["size"], Json::Null);
    let body = request_body("gen4_image", "a cat", &mapped);
    assert!(!body.contains_key("size"));
}

#[rstest]
#[case::number(number(7))]
#[case::boolean(Json::Bool(true))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn a_non_text_size_is_refused(#[case] size: Json) {
    let input = params(&[("size", size)]);
    let error = map_openai_params(&input, ImageParams::new(), "gen4_image", false).unwrap_err();
    assert!(matches!(error, RunwayImageError::InvalidSize));
    assert_eq!(error.to_string(), "the size option must be a string");
    let dropped = map_openai_params(&input, ImageParams::new(), "gen4_image", true).unwrap_err();
    assert!(matches!(dropped, RunwayImageError::InvalidSize));
}

#[rstest]
#[case::refused(false, true)]
#[case::dropped(true, false)]
fn unsupported_parameters_need_drop_params(#[case] drop_params: bool, #[case] refused: bool) {
    let input = params(&[("quality", text("hd")), ("size", text("1024x1024"))]);
    let outcome = map_openai_params(&input, ImageParams::new(), "gen4_image", drop_params);
    if refused {
        let error = outcome.unwrap_err();
        assert!(matches!(
            error,
            RunwayImageError::UnsupportedParameter { .. }
        ));
        assert_eq!(
            error.to_string(),
            "Parameter quality is not supported for model gen4_image"
        );
        return;
    }
    let mapped = outcome.unwrap();
    assert!(!mapped.contains_key("quality"));
    assert_eq!(mapped["ratio"], text("1024:1024"));
    assert_eq!(mapped["size"], text("1024x1024"));
}

#[test]
fn an_existing_option_bypasses_rejection() {
    let input = params(&[
        ("seed", number(7)),
        ("quality", text("hd")),
        ("size", text("1024x1024")),
    ]);
    let existing = params(&[("seed", number(7)), ("quality", text("hd"))]);
    let mapped = map_openai_params(&input, existing, "gen4_image", false).unwrap();
    assert_eq!(mapped["seed"], number(7));
    assert_eq!(mapped["quality"], text("hd"));
    assert_eq!(mapped["ratio"], text("1024:1024"));
    let body = request_body("gen4_image", "a cat", &mapped);
    assert_eq!(body["seed"], number(7));
    assert_eq!(body["quality"], text("hd"));
}

#[test]
fn a_supplied_size_overwrites_an_existing_ratio() {
    let input = params(&[("size", text("1024x1024"))]);
    let existing = params(&[("ratio", text("1080:1920"))]);
    let mapped = map_openai_params(&input, existing, "gen4_image", false).unwrap();
    assert_eq!(mapped["ratio"], text("1024:1024"));
}

#[test]
fn optional_values_cannot_overwrite_constructed_fields() {
    let existing = params(&[
        ("model", text("custom")),
        ("promptText", text("other")),
        ("ratio", text("1024:1024")),
        ("size", text("1024x1024")),
        ("seed", number(3)),
    ]);
    let body = request_body("gen4_image", "a cat", &existing);
    assert_eq!(body["model"], text("gen4_image"));
    assert_eq!(body["promptText"], text("a cat"));
    assert_eq!(body["ratio"], text("1024:1024"));
    assert_eq!(body["seed"], number(3));
    assert!(!body.contains_key("size"));
}

#[rstest]
#[case::empty("", "gen4_image")]
#[case::kept("gen4_image_v2", "gen4_image_v2")]
#[case::prefixed("runwayml/gen4_image", "runwayml/gen4_image")]
fn the_body_model_falls_back_or_keeps_its_name(#[case] model: &str, #[case] expected: &str) {
    let body = request_body(model, "a cat", &ImageParams::new());
    assert_eq!(body["model"], text(expected));
    assert_eq!(body["promptText"], text("a cat"));
    assert_eq!(body["ratio"], text("1920:1080"));
}

#[test]
fn string_members_append_as_urls() {
    let finished = apply_task_output(
        &task(r#"{"status":"SUCCEEDED","output":["https://o.test/a.png"]}"#),
        response(7, None),
    );
    assert_eq!(finished.created, 7);
    assert_eq!(finished.data, Some(vec![url_image("https://o.test/a.png")]));
}

#[test]
fn mapping_members_carry_url_and_b64_json() {
    let finished = apply_task_output(
        &task(
            r#"{"status":"SUCCEEDED","output":[{"url":"https://o.test/b.png","b64_json":"QUJD"}]}"#,
        ),
        response(0, None),
    );
    assert_eq!(
        finished.data,
        Some(vec![ImageData {
            b64_json: Some("QUJD".to_owned()),
            revised_prompt: None,
            url: Some("https://o.test/b.png".to_owned()),
        }])
    );
}

#[test]
fn a_mapping_without_image_fields_still_adds_an_entry() {
    let finished = apply_task_output(
        &task(r#"{"status":"SUCCEEDED","output":[{"unrelated":"x"}]}"#),
        response(0, None),
    );
    assert_eq!(finished.data, Some(vec![ImageData::default()]));
}

#[test]
fn other_members_are_skipped() {
    let finished = apply_task_output(
        &task(r#"{"status":"SUCCEEDED","output":[7,true,null,[1],1.5]}"#),
        response(3, None),
    );
    assert_eq!(finished.data, Some(Vec::new()));
    assert_eq!(finished.created, 3);
}

#[rstest]
#[case::numeric_sibling(
    r#"{"url":"https://o.test/a.png","width":1024}"#,
    Some("https://o.test/a.png"),
    None
)]
#[case::null_url(r#"{"url":null}"#, None, None)]
#[case::nested_value(r#"{"b64_json":"QUJD","meta":{"a":1}}"#, None, Some("QUJD"))]
fn a_mapping_with_a_non_string_value_still_adds_an_entry(
    #[case] member: &str,
    #[case] url: Option<&str>,
    #[case] b64_json: Option<&str>,
) {
    let body = format!(r#"{{"status":"SUCCEEDED","output":[{member}]}}"#);
    let finished = apply_task_output(&task(&body), response(0, None));
    assert_eq!(
        finished.data,
        Some(vec![ImageData {
            b64_json: b64_json.map(str::to_owned),
            revised_prompt: None,
            url: url.map(str::to_owned),
        }])
    );
}

#[rstest]
#[case::absent(r#"{"status":"SUCCEEDED"}"#)]
#[case::null(r#"{"status":"SUCCEEDED","output":null}"#)]
#[case::text(r#"{"status":"SUCCEEDED","output":"https://o.test/a.png"}"#)]
#[case::empty_list(r#"{"status":"SUCCEEDED","output":[]}"#)]
fn a_missing_or_non_list_output_is_an_empty_result(#[case] body: &str) {
    let finished = apply_task_output(&task(body), response(1700000000, None));
    assert_eq!(finished.data, Some(Vec::new()));
    assert_eq!(finished.created, 1700000000);
}

#[test]
fn existing_data_is_kept_and_appended_to() {
    let existing = ImageData {
        b64_json: None,
        revised_prompt: Some("kept".to_owned()),
        url: Some("https://o.test/old.png".to_owned()),
    };
    let finished = apply_task_output(
        &task(r#"{"status":"SUCCEEDED","output":["https://o.test/new.png"]}"#),
        response(11, Some(vec![existing.clone()])),
    );
    assert_eq!(
        finished.data,
        Some(vec![existing, url_image("https://o.test/new.png")])
    );
}

#[tokio::test]
async fn polls_the_default_host_whatever_the_submission_url() {
    let transport = Script::new(&[
        (200, r#"{"id":"task-1","status":"RUNNING"}"#),
        (
            200,
            r#"{"id":"task-1","status":"SUCCEEDED","output":["https://o.test/a.png"]}"#,
        ),
    ]);
    let clock = Clock::new(0);
    let loaded = settings(&[("RUNWAYML_API_BASE", "https://other.test")]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let finished = transform
        .transform(
            &outgoing,
            &headers(&[("authorization", "Bearer k")]),
            response(0, None),
        )
        .await
        .unwrap();
    assert_eq!(finished.data, Some(vec![url_image("https://o.test/a.png")]));
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].0,
        "https://api.dev.runwayml.com/v1/tasks/task-1"
    );
    assert_eq!(
        requests[1].0,
        "https://api.dev.runwayml.com/v1/tasks/task-1"
    );
    assert_eq!(*clock.pauses.lock().unwrap(), [Duration::from_secs(2)]);
}

#[tokio::test]
async fn forwards_only_the_original_authorization_and_version() {
    let transport = Script::new(&[(200, r#"{"id":"task-1","status":"SUCCEEDED","output":[]}"#)]);
    let clock = Clock::new(0);
    let loaded = settings(&[("RUNWAYML_DEFAULT_API_VERSION", "2025-01-01")]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let original = headers(&[
        ("authorization", "Bearer caller"),
        ("x-runway-version", "2020-02-02"),
        ("x-trace", "t-1"),
    ]);
    transform
        .transform(&outgoing, &original, response(0, None))
        .await
        .unwrap();
    let requests = transport.requests.lock().unwrap();
    let polled = &requests[0].1;
    assert_eq!(polled.len(), 2);
    assert_eq!(polled[AUTHORIZATION], "Bearer caller");
    assert!(polled[AUTHORIZATION].is_sensitive());
    assert_eq!(polled["x-runway-version"], "2020-02-02");
}

#[tokio::test]
async fn missing_request_headers_take_the_configured_defaults() {
    let transport = Script::new(&[(200, r#"{"id":"task-1","status":"SUCCEEDED","output":[]}"#)]);
    let clock = Clock::new(0);
    let loaded = settings(&[("RUNWAYML_DEFAULT_API_VERSION", "2025-01-01")]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap();
    let requests = transport.requests.lock().unwrap();
    let polled = &requests[0].1;
    assert_eq!(polled.len(), 2);
    assert_eq!(polled[AUTHORIZATION], "");
    assert_eq!(polled["x-runway-version"], "2025-01-01");
}

#[tokio::test]
async fn a_header_value_a_header_cannot_carry_is_refused() {
    let transport = Script::new(&[]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let original = headers(&[("authorization", "Bearer bad\nvalue")]);
    let error = transform
        .transform(&outgoing, &original, response(0, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RunwayImageError::Task(RunwayTaskError::InvalidHeader(_))
    ));
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_succeeded_submission_is_still_polled() {
    let transport = Script::new(&[(
        200,
        r#"{"id":"task-5","status":"SUCCEEDED","output":["https://o.test/polled.png"]}"#,
    )]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-5","status":"SUCCEEDED"}"#.to_vec(),
    };
    let finished = transform
        .transform(
            &outgoing,
            &headers(&[("authorization", "Bearer k")]),
            response(11, None),
        )
        .await
        .unwrap();
    assert_eq!(
        finished.data,
        Some(vec![url_image("https://o.test/polled.png")])
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_submission_that_is_not_json_keeps_the_status_and_headers() {
    let transport = Script::new(&[]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let original = headers(&[("x-request-id", "r-1")]);
    let outgoing = ImageHttpResponse {
        status_code: 502,
        headers: original.clone(),
        body: b"not json".to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(502));
    assert_eq!(error.headers(), Some(&original));
    assert!(matches!(
        error,
        RunwayImageError::Submission { status: 502, .. }
    ));
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[rstest]
#[case::absent(r#"{"status":"PENDING"}"#)]
#[case::empty(r#"{"id":"","status":"PENDING"}"#)]
#[tokio::test]
async fn a_submission_without_an_id_is_not_polled(#[case] body: &'static str) {
    let transport = Script::new(&[]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: body.as_bytes().to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "RunwayML response missing task ID");
    assert!(matches!(error, RunwayImageError::MissingTaskId));
    assert_eq!(error.status(), None);
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_poll_http_error_keeps_its_status() {
    let transport = Script::new(&[(500, "{}")]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RunwayImageError::Task(RunwayTaskError::HttpStatus { status: 500 })
    ));
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_poll_that_is_not_json_keeps_its_cause() {
    let transport = Script::new(&[(200, "not json")]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RunwayImageError::Task(RunwayTaskError::Decode { .. })
    ));
}

#[tokio::test]
async fn a_failed_poll_keeps_its_failure() {
    let transport = Script::new(&[(
        200,
        r#"{"id":"task-1","status":"FAILED","failure":"bad prompt","failureCode":"SAFETY"}"#,
    )]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    let RunwayImageError::Task(RunwayTaskError::Failed { kind, reason, code }) = error else {
        panic!("the poll failure should keep its cause");
    };
    assert_eq!(kind, TaskKind::Image);
    assert_eq!(reason, "bad prompt");
    assert_eq!(code, "SAFETY");
}

#[tokio::test]
async fn a_cancelled_poll_keeps_its_cause() {
    let transport = Script::new(&[(200, r#"{"id":"task-1","status":"CANCELLED"}"#)]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RunwayImageError::Task(RunwayTaskError::Cancelled {
            kind: TaskKind::Image
        })
    ));
}

#[tokio::test]
async fn a_poll_loop_uses_the_configured_timeout() {
    let running = (200, r#"{"id":"task-1","status":"RUNNING"}"#);
    let transport = Script::new(&[running; 8]);
    let clock = Clock::new(1001);
    let loaded = settings(&[("RUNWAYML_POLLING_TIMEOUT", "1")]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-1","status":"PENDING"}"#.to_vec(),
    };
    let error = transform
        .transform(&outgoing, &ImageHeaders::new(), response(0, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RunwayImageError::Task(RunwayTaskError::TimedOut { seconds: 1, .. })
    ));
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_transformed_response_keeps_the_existing_created_and_data() {
    let transport = Script::new(&[(
        200,
        r#"{"id":"task-2","status":"SUCCEEDED","output":["https://o.test/new.png"]}"#,
    )]);
    let clock = Clock::new(0);
    let loaded = settings(&[]);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &loaded,
    };
    let existing = ImageData {
        b64_json: None,
        revised_prompt: Some("kept".to_owned()),
        url: None,
    };
    let outgoing = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: br#"{"id":"task-2","status":"PENDING"}"#.to_vec(),
    };
    let finished = transform
        .transform(
            &outgoing,
            &ImageHeaders::new(),
            response(1700000000, Some(vec![existing.clone()])),
        )
        .await
        .unwrap();
    assert_eq!(finished.created, 1700000000);
    assert_eq!(
        finished.data,
        Some(vec![existing, url_image("https://o.test/new.png")])
    );
}

fn priced_record(json: &str) -> PriceRecord {
    serde_json::from_str(json).unwrap()
}

#[derive(Default)]
struct Catalog {
    entries: BTreeMap<String, PriceRecord>,
    seen: Mutex<Vec<(String, Option<String>)>>,
}

impl Catalog {
    fn new(entries: &[(&str, &str)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(model, json)| ((*model).to_owned(), priced_record(json)))
                .collect(),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl PriceLookup for Catalog {
    fn effective_model_info(&self, model: &str, provider: Option<&str>) -> Option<PriceRecord> {
        self.seen
            .lock()
            .unwrap()
            .push((model.to_owned(), provider.map(str::to_owned)));
        self.entries.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(&self, model: &str, provider: Option<&str>) -> Option<PricedModel> {
        self.effective_model_info(model, provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

fn image_call<'request>(
    billing: &'request ImageBilling,
    prices: Option<&'request PriceRecord>,
) -> PricedImage<'request> {
    PricedImage {
        model: "gen4_image",
        response: ImageResponseView::Image(billing),
        prices,
    }
}

fn billing(image_count: i64) -> ImageBilling {
    ImageBilling {
        image_count,
        size: None,
        quality: None,
        usage: None,
    }
}

#[rstest]
#[case::three(3, "1.5")]
#[case::one(1, "0.5")]
#[case::none(0, "0")]
fn the_charge_multiplies_returned_entries(#[case] count: i64, #[case] expected: &str) {
    let record = priced_record(r#"{"output_cost_per_image":0.5}"#);
    let cost = cost_calculator(
        &image_call(&billing(count), Some(&record)),
        &Catalog::default(),
    )
    .unwrap();
    assert_eq!(cost.normalize().to_string(), expected);
}

#[rstest]
#[case::explicit_zero(r#"{"output_cost_per_image":0}"#)]
#[case::null_price(r#"{"output_cost_per_image":null}"#)]
#[case::absent(r#"{"key":"gen4_image"}"#)]
fn a_missing_or_zero_price_costs_nothing(#[case] json: &str) {
    let record = priced_record(json);
    let cost =
        cost_calculator(&image_call(&billing(4), Some(&record)), &Catalog::default()).unwrap();
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn the_price_lookup_sees_the_runwayml_provider() {
    let catalog = Catalog::new(&[("gen4_image", r#"{"output_cost_per_image":0.25}"#)]);
    let cost = cost_calculator(&image_call(&billing(4), None), &catalog).unwrap();
    assert_eq!(cost, Decimal::new(1, 0));
    let seen = catalog.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "gen4_image");
    assert_eq!(seen[0].1.as_deref(), Some(CUSTOM_LLM_PROVIDER));
}

#[test]
fn deployment_prices_overlay_the_catalog() {
    let catalog = Catalog::new(&[("gen4_image", r#"{"output_cost_per_image":9}"#)]);
    let record = priced_record(r#"{"output_cost_per_image":0.5}"#);
    let cost = cost_calculator(&image_call(&billing(2), Some(&record)), &catalog).unwrap();
    assert_eq!(cost, Decimal::new(1, 0));
}

#[test]
fn the_requested_size_and_quality_do_not_change_the_charge() {
    let record = priced_record(r#"{"output_cost_per_image":2.5}"#);
    let shaped = ImageBilling {
        image_count: 2,
        size: Some("1920x1080".to_owned()),
        quality: Some("hd".to_owned()),
        usage: None,
    };
    let plain =
        cost_calculator(&image_call(&billing(2), Some(&record)), &Catalog::default()).unwrap();
    let with_shape =
        cost_calculator(&image_call(&shaped, Some(&record)), &Catalog::default()).unwrap();
    assert_eq!(with_shape, plain);
    assert_eq!(with_shape, Decimal::new(5, 0));
}

#[rstest]
#[case::zero(r#"{"output_cost_per_image":0}"#)]
#[case::absent(r#"{}"#)]
fn a_non_image_response_is_refused(#[case] json: &str) {
    let record = priced_record(json);
    let call = PricedImage {
        model: "gen4_image",
        response: ImageResponseView::Other,
        prices: Some(&record),
    };
    let error = cost_calculator(&call, &Catalog::default()).unwrap_err();
    assert!(matches!(error, ImageCostError::NotImageResponse));
}

#[test]
fn an_unknown_model_has_no_price() {
    let call = PricedImage {
        model: "gen4_image",
        response: ImageResponseView::Other,
        prices: None,
    };
    let error = cost_calculator(&call, &Catalog::default()).unwrap_err();
    assert!(matches!(error, ImageCostError::UnknownModel { .. }));
}

#[test]
fn an_overflowing_charge_is_refused() {
    let record = priced_record(r#"{"output_cost_per_image":10000000000}"#);
    let error = cost_calculator(
        &image_call(&billing(i64::MAX), Some(&record)),
        &Catalog::default(),
    )
    .unwrap_err();
    assert!(matches!(error, ImageCostError::Overflow));
}

#[test]
fn a_text_price_names_the_field() {
    let record = priced_record(r#"{"output_cost_per_image":"0.5"}"#);
    let error =
        cost_calculator(&image_call(&billing(2), Some(&record)), &Catalog::default()).unwrap_err();
    assert!(matches!(error, ImageCostError::WrongFieldType { .. }));
}
