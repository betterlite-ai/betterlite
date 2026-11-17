#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Number;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;
use url::Url;

use super::poll::{TokioSleeper, decode_assembly_transcript};
use super::{
    AssemblyAiError, AssemblyAiLogging, AssemblyAiPorts, AssemblyAiSettings,
    AssemblyCredentialSource, AssemblyLogCall, AssemblySleeper, AssemblyTranscriptLog,
    AssemblyTranscriptLogSink, AssemblyTranscriptTransport, assembly_base_url_from_region,
    assembly_region_from_url,
};

const BEST_RATE: &str = "3.333e-05";
const NANO_RATE: &str = "0.00010278";
const START: &str = "2026-01-01T00:00:00Z";
const END: &str = "2026-01-01T00:00:01Z";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

const fn settings(max_polling_attempts: u32) -> AssemblyAiSettings {
    AssemblyAiSettings {
        polling_base_url: "https://api.assemblyai.com",
        polling_eu_base_url: "https://eu.assemblyai.com",
        polling_interval: Duration::from_secs(10),
        max_polling_attempts,
    }
}

fn body_with(transcript_id: &str, speech_model: &str) -> Json {
    let id = serde_json::to_string(transcript_id).unwrap();
    let model = serde_json::to_string(speech_model).unwrap();
    json(&format!(r#"{{"id":{id},"speech_model":{model}}}"#))
}

fn log_call(request_method: &str, response_body: Json, url_route: Option<&str>) -> AssemblyLogCall {
    AssemblyLogCall {
        request_method: request_method.to_owned(),
        response_body,
        url_route: url_route.map(str::to_owned),
        result: "success".to_owned(),
        start_time: at(START),
        end_time: at(END),
        cache_hit: false,
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

#[derive(Clone)]
enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Failure,
}

fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer {
        status,
        body: serde_json::to_vec(&json(body)).unwrap(),
    }
}

fn queued() -> Reply {
    answer(200, r#"{"id":"t1","status":"queued"}"#)
}

fn completed_transcript(duration: &str) -> Reply {
    answer(
        200,
        &format!(r#"{{"id":"t1","status":"completed","audio_duration":{duration}}}"#),
    )
}

#[derive(Clone)]
struct Recorded {
    url: Url,
    headers: HeaderMap,
}

#[derive(Clone)]
struct Transport {
    script: Arc<Mutex<VecDeque<Reply>>>,
    repeat: Reply,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Transport {
    fn new(replies: Vec<Reply>) -> Self {
        let repeat = replies.last().cloned().unwrap_or_else(queued);
        Self {
            script: Arc::new(Mutex::new(VecDeque::from(replies))),
            repeat,
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        lock(&self.requests).clone()
    }

    fn request_count(&self) -> usize {
        lock(&self.requests).len()
    }
}

impl AssemblyTranscriptTransport for Transport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        lock(&self.requests).push(Recorded { url, headers });
        let reply = lock(&self.script)
            .pop_front()
            .unwrap_or_else(|| self.repeat.clone());
        ready(match reply {
            Reply::Answer { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Reply::Failure => Err(TransportError::Closed),
        })
    }
}

type CredentialCalls = Arc<Mutex<Vec<(String, Option<String>)>>>;

#[derive(Clone)]
struct Credentials {
    key: Option<SecretString>,
    calls: CredentialCalls,
}

impl Credentials {
    fn new(key: Option<&str>) -> Self {
        Self {
            key: key.map(SecretString::from),
            calls: Arc::default(),
        }
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        lock(&self.calls).clone()
    }
}

impl AssemblyCredentialSource for Credentials {
    fn get_credentials(
        &self,
        custom_llm_provider: &str,
        region_name: Option<&str>,
    ) -> impl Future<Output = Option<SecretString>> + Send {
        lock(&self.calls).push((
            custom_llm_provider.to_owned(),
            region_name.map(str::to_owned),
        ));
        ready(self.key.clone())
    }
}

#[derive(Clone, Default)]
struct Sleeper {
    slept: Arc<Mutex<Vec<Duration>>>,
}

impl Sleeper {
    fn sleeps(&self) -> Vec<Duration> {
        lock(&self.slept).clone()
    }
}

impl AssemblySleeper for Sleeper {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        lock(&self.slept).push(duration);
        ready(())
    }
}

#[derive(Clone, Default)]
struct Pricing {
    catalog: Arc<BTreeMap<String, PriceRecord>>,
}

impl Pricing {
    fn rate(&mut self, key: &str, value: Scalar) {
        let mut record = PriceRecord::new();
        record.insert("input_cost_per_second".to_owned(), value);
        Arc::make_mut(&mut self.catalog).insert(key.to_owned(), record);
    }

    fn entry(&self, model: &str, custom_llm_provider: Option<&str>) -> Option<PriceRecord> {
        let scoped = custom_llm_provider.map(|provider| format!("{provider}/{model}"));
        scoped
            .and_then(|key| self.catalog.get(&key).cloned())
            .or_else(|| self.catalog.get(model).cloned())
    }
}

impl PriceLookup for Pricing {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.entry(model, custom_llm_provider)
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.catalog.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.entry(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

fn catalog(entries: &[(&str, &str)]) -> Pricing {
    let mut price = Pricing::default();
    for (key, value) in entries {
        price.rate(key, Scalar::Number(Number::from_str(value).unwrap()));
    }
    price
}

#[derive(Clone, Default)]
struct Sink {
    calls: Arc<Mutex<Vec<AssemblyTranscriptLog>>>,
}

impl Sink {
    fn count(&self) -> usize {
        lock(&self.calls).len()
    }

    fn single(&self) -> AssemblyTranscriptLog {
        let mut calls = lock(&self.calls);
        assert_eq!(calls.len(), 1);
        calls.remove(0)
    }
}

impl AssemblyTranscriptLogSink for Sink {
    fn handle_logging(&self, call: AssemblyTranscriptLog) -> impl Future<Output = ()> + Send {
        lock(&self.calls).push(call);
        ready(())
    }
}

struct Harness {
    credentials: Credentials,
    transport: Transport,
    pricing: Pricing,
    sleeper: Sleeper,
    sink: Sink,
    tasks: TaskTracker,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            credentials: Credentials::new(Some("secret-key")),
            transport: Transport::new(vec![completed_transcript("12.5")]),
            pricing: catalog(&[
                ("assemblyai/best", BEST_RATE),
                ("assemblyai/nano", NANO_RATE),
            ]),
            sleeper: Sleeper::default(),
            sink: Sink::default(),
            tasks: TaskTracker::new(),
        }
    }
}

type Service = AssemblyAiLogging<Credentials, Transport, Pricing, Sleeper, Sink>;

impl Harness {
    fn with_credentials(key: Option<&str>) -> Self {
        Self {
            credentials: Credentials::new(key),
            transport: Transport::new(vec![completed_transcript("12.5")]),
            pricing: catalog(&[
                ("assemblyai/best", BEST_RATE),
                ("assemblyai/nano", NANO_RATE),
            ]),
            sleeper: Sleeper::default(),
            sink: Sink::default(),
            tasks: TaskTracker::new(),
        }
    }

    fn with_pricing(pricing: Pricing) -> Self {
        Self {
            credentials: Credentials::new(Some("secret-key")),
            transport: Transport::new(vec![completed_transcript("12.5")]),
            pricing,
            sleeper: Sleeper::default(),
            sink: Sink::default(),
            tasks: TaskTracker::new(),
        }
    }

    fn set_transport(&mut self, replies: Vec<Reply>) {
        self.transport = Transport::new(replies);
    }

    fn set_pricing(&mut self, entries: &[(&str, &str)]) {
        self.pricing = catalog(entries);
    }

    fn service(&self, settings: AssemblyAiSettings) -> Arc<Service> {
        Arc::new(AssemblyAiLogging::new(
            AssemblyAiPorts {
                credentials: self.credentials.clone(),
                transport: self.transport.clone(),
                pricing: self.pricing.clone(),
                sleeper: self.sleeper.clone(),
                sink: self.sink.clone(),
            },
            settings,
            self.tasks.clone(),
        ))
    }

    async fn run_with(
        &self,
        settings: AssemblyAiSettings,
        body: Json,
        url_route: Option<&str>,
    ) -> Result<(), AssemblyAiError> {
        let service = self.service(settings);
        service.run(&log_call("POST", body, url_route)).await
    }

    async fn run(&self, body: Json, url_route: Option<&str>) -> Result<(), AssemblyAiError> {
        self.run_with(settings(2), body, url_route).await
    }
}

#[test]
fn default_settings_match_litellm() {
    let defaults = AssemblyAiSettings::new();
    assert_eq!(
        (defaults.polling_base_url, defaults.polling_eu_base_url),
        ("https://api.assemblyai.com", "https://eu.assemblyai.com")
    );
    assert_eq!(
        (defaults.polling_interval, defaults.max_polling_attempts),
        (Duration::from_secs(10), 180)
    );
    assert_eq!(
        (
            defaults.polling_base(None),
            defaults.polling_base(Some("us")),
            defaults.polling_base(Some("eu"))
        ),
        (
            "https://api.assemblyai.com",
            "https://api.assemblyai.com",
            "https://eu.assemblyai.com"
        )
    );
    assert_eq!(
        AssemblyAiSettings::default().polling_base(None),
        defaults.polling_base_url
    );
}

#[rstest::rstest]
#[case::absent(None, None)]
#[case::exact(Some("https://eu.assemblyai.com/v2/transcript"), Some("eu"))]
#[case::upper_case(Some("https://EU.AssemblyAI.com/v2/transcript"), Some("eu"))]
#[case::userinfo_and_port(Some("https://user:pw@eu.assemblyai.com:443/x"), Some("eu"))]
#[case::scheme_relative(Some("//eu.assemblyai.com/x"), Some("eu"))]
#[case::padded(Some("  https://eu.assemblyai.com/x"), Some("eu"))]
#[case::api_eu_host(Some("https://api.eu.assemblyai.com/v2/transcript"), None)]
#[case::suffix_host(Some("https://eu.assemblyai.com.evil.test/x"), None)]
#[case::us_host(Some("https://api.assemblyai.com/v2/transcript"), None)]
#[case::no_scheme(Some("eu.assemblyai.com/v2/transcript"), None)]
#[case::empty(Some(""), None)]
#[case::ipv6(Some("https://[::1]/x"), None)]
#[case::bracketed_eu(Some("https://[eu.assemblyai.com]/x"), None)]
#[case::interior_tab(Some("https://eu.assemblyai.com\t/x"), Some("eu"))]
#[case::interior_newline(Some("https://eu.assemblyai.com\n/x"), Some("eu"))]
#[case::host_before_space(Some("https://eu.assemblyai.com /x"), None)]
#[case::nfkc_rejected(Some("https://eu.assemblyai.com\u{2100}/x"), None)]
#[case::numeric_scheme(Some("1https://eu.assemblyai.com/x"), None)]
#[case::empty_authority(Some("https:///x"), None)]
fn assembly_region_follows_the_url_host(#[case] url: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(assembly_region_from_url(url), expected, "{url:?}");
}

#[test]
fn assembly_base_url_maps_only_eu_to_the_routing_eu_host() {
    assert_eq!(
        assembly_base_url_from_region(Some("eu")),
        "https://api.eu.assemblyai.com"
    );
    assert_eq!(
        assembly_base_url_from_region(None),
        "https://api.assemblyai.com"
    );
    assert_eq!(
        assembly_base_url_from_region(Some("us")),
        "https://api.assemblyai.com"
    );
}

#[tokio::test]
async fn post_job_polls_to_completion() {
    let mut harness = Harness::default();
    harness.set_transport(vec![queued(), queued(), completed_transcript("12.5")]);
    harness
        .run_with(AssemblyAiSettings::new(), body_with("t1", "best"), None)
        .await
        .unwrap();
    let requests = harness.transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].url.as_str(),
        "https://api.assemblyai.com/v2/transcript/t1"
    );
    assert_eq!(
        (
            requests[2].headers["authorization"].to_str().unwrap(),
            requests[2].headers["content-type"].to_str().unwrap()
        ),
        ("Bearer secret-key", "application/json")
    );
    assert_eq!(
        harness.credentials.calls(),
        vec![("assemblyai".to_owned(), None); 3]
    );
    assert_eq!(harness.sleeper.sleeps(), vec![Duration::from_secs(10); 2]);
}

#[tokio::test]
async fn completed_job_logs_spend_and_the_callback_payload() {
    let harness = Harness::default();
    harness.run(body_with("t1", "best"), None).await.unwrap();
    let logged = harness.sink.single();
    assert_eq!(
        (
            logged.spend.model.as_str(),
            logged.spend.custom_llm_provider
        ),
        ("best", "assemblyai")
    );
    assert_eq!(logged.spend.response_cost, Some(decimal("0.000416625")));
    assert_eq!(
        logged.response,
        PassThroughResponse::Transcription(json(
            r#"{"id":"t1","status":"completed","audio_duration":12.5}"#
        ))
    );
    assert_eq!(
        (
            logged.result.as_str(),
            logged.start_time,
            logged.end_time,
            logged.cache_hit
        ),
        ("success", at(START), at(END), false)
    );
    assert_eq!(
        logged.kwargs.entries.get("model"),
        Some(&Json::Text("best".to_owned()))
    );
    assert_eq!(
        logged.kwargs.entries.get("custom_llm_provider"),
        Some(&Json::Text("assemblyai".to_owned()))
    );
    assert!(!logged.kwargs.entries.contains_key("response_cost"));
}

#[tokio::test]
async fn polling_stops_on_the_error_status() {
    let mut harness = Harness::default();
    harness.set_transport(vec![answer(200, r#"{"id":"t1","status":"error"}"#)]);
    harness.run(body_with("t1", "best"), None).await.unwrap();
    assert_eq!(harness.transport.request_count(), 1);
    assert!(harness.sleeper.sleeps().is_empty());
    let logged = harness.sink.single();
    assert_eq!(logged.spend.response_cost, None);
    assert_eq!(
        logged.response,
        PassThroughResponse::Transcription(json(r#"{"id":"t1","status":"error"}"#))
    );
}

#[tokio::test]
async fn polling_gives_up_after_the_maximum_attempts() {
    let mut harness = Harness::default();
    harness.set_transport(vec![queued()]);
    harness
        .run_with(AssemblyAiSettings::new(), body_with("t1", "best"), None)
        .await
        .unwrap();
    assert_eq!(harness.transport.request_count(), 180);
    assert_eq!(harness.sleeper.sleeps(), vec![Duration::from_secs(10); 180]);
    let logged = harness.sink.single();
    assert_eq!(
        logged.response,
        PassThroughResponse::Transcription(json("{}"))
    );
    assert_eq!(logged.spend.response_cost, None);
}

#[rstest::rstest]
#[case::server_error(Some((500, r#"{"error":"boom"}"#)))]
#[case::client_error(Some((400, r#"{"status":"completed","audio_duration":5}"#)))]
#[case::not_json(Some((200, "{not json")))]
#[case::null_body(Some((200, "null")))]
#[case::transport_failure(None)]
#[tokio::test]
async fn absent_transcript_stops_at_once_and_logs_an_empty_response(
    #[case] reply: Option<(u16, &str)>,
) {
    let mut harness = Harness::default();
    harness.set_transport(vec![match reply {
        Some((status, body)) => Reply::Answer {
            status,
            body: body.as_bytes().to_vec(),
        },
        None => Reply::Failure,
    }]);
    harness.run(body_with("t1", "best"), None).await.unwrap();
    assert_eq!(harness.transport.request_count(), 1);
    assert!(harness.sleeper.sleeps().is_empty());
    let logged = harness.sink.single();
    assert_eq!(
        logged.response,
        PassThroughResponse::Transcription(json("{}"))
    );
    assert_eq!(logged.spend.response_cost, None);
}

#[rstest::rstest]
#[case::absent(json(r#"{"speech_model":"best"}"#))]
#[case::non_object(json("[1]"))]
#[tokio::test]
async fn a_missing_transcript_id_fails_without_polling(#[case] body: Json) {
    let harness = Harness::default();
    let error = harness.run(body, None).await.unwrap_err();
    assert!(matches!(error, AssemblyAiError::MissingTranscriptId));
    assert_eq!(
        error.to_string(),
        "Transcript ID is required to log the cost of the transcription"
    );
    assert_eq!(harness.transport.request_count(), 0);
    assert_eq!(harness.sink.count(), 0);
}

#[tokio::test]
async fn a_missing_api_key_fails_without_polling() {
    let harness = Harness::with_credentials(None);
    let error = harness
        .run(body_with("t1", "best"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, AssemblyAiError::MissingApiKey));
    assert_eq!(error.to_string(), "AssemblyAI API key not found");
    assert_eq!(
        harness.credentials.calls(),
        vec![("assemblyai".to_owned(), None)]
    );
    assert_eq!(harness.transport.request_count(), 0);
    assert_eq!(harness.sink.count(), 0);
}

#[rstest::rstest]
#[case::slash("a/b")]
#[case::backslash(r"a\b")]
#[case::hash("a#b")]
#[case::question("a?b")]
#[case::parent("a..b")]
#[tokio::test]
async fn disallowed_transcript_ids_are_rejected_before_any_request(#[case] transcript_id: &str) {
    let harness = Harness::default();
    let error = harness
        .run(body_with(transcript_id, "best"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, AssemblyAiError::InvalidTranscriptId { .. }),
        "{transcript_id:?} was accepted"
    );
    assert_eq!(harness.transport.request_count(), 0);
    assert_eq!(harness.sink.count(), 0);
}

#[rstest::rstest]
#[case::space("t 1", "/v2/transcript/t%201")]
#[case::unicode("t\u{e9}st", "/v2/transcript/t%C3%A9st")]
#[case::colon("a:b", "/v2/transcript/a%3Ab")]
#[case::tilde("a~b", "/v2/transcript/a~b")]
#[tokio::test]
async fn transcript_ids_are_percent_encoded_in_the_path(
    #[case] transcript_id: &str,
    #[case] path: &str,
) {
    let harness = Harness::default();
    harness
        .run(body_with(transcript_id, "best"), None)
        .await
        .unwrap();
    assert_eq!(
        harness.transport.requests()[0].url.path(),
        path,
        "{transcript_id:?}"
    );
}

#[rstest::rstest]
#[case::eu(
    "https://eu.assemblyai.com/v2/transcript",
    "eu.assemblyai.com",
    Some("eu")
)]
#[case::api_eu(
    "https://api.eu.assemblyai.com/v2/transcript",
    "api.assemblyai.com",
    None
)]
#[tokio::test]
async fn eu_url_route_polls_the_eu_base_and_api_eu_keeps_the_default(
    #[case] url_route: &str,
    #[case] host: &str,
    #[case] region: Option<&str>,
) {
    let harness = Harness::default();
    harness
        .run(body_with("t1", "best"), Some(url_route))
        .await
        .unwrap();
    let request = &harness.transport.requests()[0];
    assert_eq!(request.url.host_str(), Some(host));
    assert_eq!(request.url.path(), "/v2/transcript/t1");
    assert_eq!(
        harness.credentials.calls(),
        vec![("assemblyai".to_owned(), region.map(str::to_owned))]
    );
}

#[rstest::rstest]
#[case::exact("best", "12.5", "0.000416625")]
#[case::fallback("universal", "3600.5", "0.37005939")]
#[case::empty_model("", "100", "0.010278")]
#[case::prefixed_nano("assemblyai/nano", "2.5", "0.00025695")]
#[tokio::test]
async fn pricing_uses_the_exact_model_rate_and_falls_back_to_nano(
    #[case] speech_model: &str,
    #[case] duration: &str,
    #[case] expected: &str,
) {
    let mut harness = Harness::default();
    harness.set_transport(vec![completed_transcript(duration)]);
    harness
        .run(body_with("t1", speech_model), None)
        .await
        .unwrap();
    let logged = harness.sink.single();
    assert_eq!(logged.spend.model, speech_model);
    assert_eq!(logged.spend.response_cost, Some(decimal(expected)));
}

#[rstest::rstest]
#[case::absent(r#"{"id":"t1","status":"completed"}"#, None)]
#[case::null_duration(r#"{"id":"t1","status":"completed","audio_duration":null}"#, None)]
#[case::zero_duration(r#"{"id":"t1","status":"completed","audio_duration":0}"#, Some("0"))]
#[tokio::test]
async fn absent_and_zero_audio_durations_differ_in_spend(
    #[case] transcript: &str,
    #[case] expected: Option<&str>,
) {
    let mut harness = Harness::default();
    harness.set_transport(vec![answer(200, transcript)]);
    harness.run(body_with("t1", "best"), None).await.unwrap();
    let logged = harness.sink.single();
    assert_eq!(logged.spend.response_cost, expected.map(decimal));
}

#[rstest::rstest]
#[case::exact_only(&[("assemblyai/best", BEST_RATE)], None)]
#[case::nano_only(&[("assemblyai/nano", NANO_RATE)], Some("0.00010278"))]
#[case::empty(&[], None)]
#[tokio::test]
async fn unusable_second_rates_leave_the_spend_without_a_cost(
    #[case] entries: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let mut harness = Harness::default();
    harness.set_pricing(entries);
    harness.set_transport(vec![completed_transcript("1")]);
    harness
        .run(body_with("t1", "universal"), None)
        .await
        .unwrap();
    let logged = harness.sink.single();
    assert_eq!(logged.spend.response_cost, expected.map(decimal));
}

#[tokio::test]
async fn a_text_second_rate_is_an_error() {
    let mut pricing = Pricing::default();
    pricing.rate("assemblyai/best", Scalar::Text("abc".to_owned()));
    let harness = Harness::with_pricing(pricing);
    let error = harness
        .run(body_with("t1", "best"), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AssemblyAiError::NumberField {
            field: "input_cost_per_second"
        }
    ));
    assert_eq!(harness.sink.count(), 0);
}

#[tokio::test]
async fn a_text_audio_duration_is_an_error() {
    let mut harness = Harness::default();
    harness.set_transport(vec![answer(
        200,
        r#"{"id":"t1","status":"completed","audio_duration":"12.5"}"#,
    )]);
    let error = harness
        .run(body_with("t1", "best"), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AssemblyAiError::NumberField {
            field: "audio_duration"
        }
    ));
}

#[rstest::rstest]
#[case::numeric_id(json(r#"{"id":7}"#), "id")]
#[case::numeric_model(json(r#"{"id":"t1","speech_model":7}"#), "speech_model")]
#[tokio::test]
async fn wrong_typed_body_fields_name_the_field(#[case] body: Json, #[case] field: &str) {
    let harness = Harness::default();
    let error = harness.run(body, None).await.unwrap_err();
    assert!(matches!(error, AssemblyAiError::TextField { field: found } if found == field));
    assert_eq!(harness.transport.request_count(), 0);
}

#[tokio::test]
async fn a_wrong_typed_status_names_the_field() {
    let mut harness = Harness::default();
    harness.set_transport(vec![answer(200, r#"{"status":5}"#)]);
    let error = harness
        .run(body_with("t1", "best"), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AssemblyAiError::TextField { field: "status" }
    ));
    assert_eq!(harness.sink.count(), 0);
}

#[tokio::test]
async fn a_non_object_transcript_is_an_error() {
    let mut harness = Harness::default();
    harness.set_transport(vec![answer(200, "[1]")]);
    let error = harness
        .run(body_with("t1", "best"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, AssemblyAiError::TranscriptShape));
    assert_eq!(harness.sink.count(), 0);
}

#[tokio::test]
async fn a_base_url_that_does_not_parse_logs_an_empty_response() {
    let harness = Harness::default();
    let service = harness.service(AssemblyAiSettings {
        polling_base_url: "not a url",
        polling_eu_base_url: "not a url",
        polling_interval: Duration::from_secs(10),
        max_polling_attempts: 2,
    });
    service
        .run(&log_call("POST", body_with("t1", "best"), None))
        .await
        .unwrap();
    assert_eq!(harness.transport.request_count(), 0);
    assert_eq!(
        harness.sink.single().response,
        PassThroughResponse::Transcription(json("{}"))
    );
}

#[tokio::test]
async fn an_api_key_that_cannot_be_a_header_logs_an_empty_response() {
    let harness = Harness::with_credentials(Some("bad\nkey"));
    harness.run(body_with("t1", "best"), None).await.unwrap();
    assert_eq!(harness.transport.request_count(), 0);
    assert_eq!(
        harness.sink.single().response,
        PassThroughResponse::Transcription(json("{}"))
    );
}

#[tokio::test]
async fn an_overflowing_cost_is_an_error() {
    let mut harness = Harness::default();
    harness.set_pricing(&[("assemblyai/best", "1e28")]);
    harness.set_transport(vec![answer(
        200,
        r#"{"id":"t1","status":"completed","audio_duration":1e28}"#,
    )]);
    let error = harness
        .run(body_with("t1", "best"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, AssemblyAiError::CostOverflow));
}

#[rstest::rstest]
#[case::post("POST", 1)]
#[case::get("GET", 0)]
#[case::put("PUT", 0)]
#[case::lowercase("post", 0)]
#[case::empty("", 0)]
#[tokio::test]
async fn only_post_requests_are_logged(#[case] method: &str, #[case] expected: usize) {
    let harness = Harness::default();
    let service = harness.service(settings(2));
    service.log(log_call(method, body_with("t1", "best"), None));
    harness.tasks.close();
    harness.tasks.wait().await;
    assert_eq!(
        (harness.sink.count(), harness.transport.request_count()),
        (expected, expected)
    );
}

#[tokio::test(start_paused = true)]
async fn the_tokio_sleeper_waits_for_the_whole_interval() {
    let waited = timeout(
        Duration::from_millis(1),
        TokioSleeper.sleep(Duration::from_secs(60)),
    )
    .await;
    assert!(waited.is_err());
}

#[test]
fn logging_outside_a_runtime_skips_the_poll() {
    let harness = Harness::default();
    let service = harness.service(settings(2));
    service.log(log_call("POST", body_with("t1", "best"), None));
    assert_eq!(
        (harness.sink.count(), harness.transport.request_count()),
        (0, 0)
    );
}

#[rstest::rstest]
fn replays_the_assembly_transcript_corpus(
    #[files("fuzz/corpus/assemblyai_transcript/*")] path: PathBuf,
) {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned();
    let bytes = fs::read(&path).expect("a corpus file is readable");
    let outcome = decode_assembly_transcript(&bytes);
    if name.starts_with("rejected-") {
        assert!(
            matches!(outcome, Err(AssemblyAiError::TranscriptShape)),
            "{name}"
        );
        return;
    }
    if name.starts_with("ok-tolerated-") {
        assert!(matches!(outcome, Ok(None)), "{name}");
        return;
    }
    assert!(matches!(outcome, Ok(Some(_))), "{name}");
}
