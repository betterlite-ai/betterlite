#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use futures_util::stream;
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::live::exchange::{
    BodyStream, HttpAnswer, HttpMethod, HttpTransport, OutboundHttp, header_pairs, signed_headers,
};
use crate::live::probe::download_and_measure;
use crate::live::signing::{AwsSigner, ClockedSigner, CredentialSource, signed_get};
use crate::live::{ProbeError, S3MediaProbe, TranscribeJobLookup, price_fail_closed};
use crate::media_pricing::{JobPricing, MediaDurationProbe, PricingError};
use crate::media_source::MediaSizeGuard;
use crate::policy::cost::TranscribeCostError;
use crate::polling::PollSleeper;
use crate::records::{GetTranscriptionJobRequest, JobLookup, JobLookupError, JobLookupFuture};

const VALID_WAV: &[u8] = include_bytes!("../../fuzz/corpus/media_duration/valid.wav");
const COMPLETED_WITH_MEDIA: &str = r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","CreationTime":100.0,"Media":{"MediaFileUri":"s3://audio/a.wav"}}}"#;
const COMPLETED_WITHOUT_MEDIA: &str =
    r#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED","CreationTime":100.0}}"#;
const BEFORE_CREATION: &str = "Thu, 01 Jan 1970 00:01:40 GMT";
const AFTER_CREATION: &str = "Thu, 01 Jan 1970 00:03:20 GMT";

fn stream_of(chunks: Vec<Vec<u8>>) -> BodyStream {
    Box::pin(stream::iter(
        chunks.into_iter().map(Ok::<Vec<u8>, ProbeError>),
    ))
}

fn answer(status: u16, headers: &[(&str, &str)], body: &[u8]) -> HttpAnswer {
    HttpAnswer {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: stream_of(vec![body.to_vec()]),
    }
}

#[derive(Clone)]
struct PostCall {
    region: String,
    url: String,
    body: String,
    headers: Vec<(String, String)>,
}

#[derive(Clone)]
struct GetCall {
    region: String,
    url: String,
}

#[derive(Clone)]
struct SignerLog {
    posts: Arc<Mutex<Vec<PostCall>>>,
    gets: Arc<Mutex<Vec<GetCall>>>,
}

impl SignerLog {
    fn new() -> Self {
        Self {
            posts: Arc::new(Mutex::new(Vec::new())),
            gets: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn posts(&self) -> Vec<PostCall> {
        self.posts.lock().unwrap().clone()
    }

    fn gets(&self) -> Vec<GetCall> {
        self.gets.lock().unwrap().clone()
    }
}

fn signed_request(
    method: &str,
    url: &str,
    headers: Vec<(String, String)>,
    body: &[u8],
) -> AwsSignedRequest {
    AwsSignedRequest {
        method: method.to_owned(),
        url: url.to_owned(),
        headers,
        body: body.to_vec(),
    }
}

struct RecordingSigner {
    log: SignerLog,
    fail: bool,
}

#[async_trait]
impl AwsSigner for RecordingSigner {
    async fn sign_json_post(
        &self,
        region: &str,
        url: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<AwsSignedRequest, ProbeError> {
        if self.fail {
            return Err(ProbeError::TaskClosed);
        }
        self.log.posts.lock().unwrap().push(PostCall {
            region: region.to_owned(),
            url: url.to_owned(),
            body: body.to_owned(),
            headers: headers.to_vec(),
        });
        Ok(signed_request(
            "POST",
            url,
            headers.to_vec(),
            body.as_bytes(),
        ))
    }

    async fn sign_s3_get(&self, region: &str, url: &str) -> Result<AwsSignedRequest, ProbeError> {
        if self.fail {
            return Err(ProbeError::TaskClosed);
        }
        self.log.gets.lock().unwrap().push(GetCall {
            region: region.to_owned(),
            url: url.to_owned(),
        });
        let authorization = (
            "Authorization".to_owned(),
            "AWS4-HMAC-SHA256 signed".to_owned(),
        );
        Ok(signed_request("GET", url, vec![authorization], b""))
    }
}

fn signer() -> (RecordingSigner, SignerLog) {
    let log = SignerLog::new();
    (
        RecordingSigner {
            log: log.clone(),
            fail: false,
        },
        log,
    )
}

#[derive(Clone)]
struct RecordedRequest {
    method: HttpMethod,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct RequestLog(Arc<Mutex<Vec<RecordedRequest>>>);

impl RequestLog {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    fn sent(&self) -> Vec<RecordedRequest> {
        self.0.lock().unwrap().clone()
    }

    fn record(&self, request: RecordedRequest) {
        self.0.lock().unwrap().push(request);
    }
}

struct ScriptedHttp {
    requests: RequestLog,
    replies: Mutex<VecDeque<Result<HttpAnswer, ProbeError>>>,
}

impl ScriptedHttp {
    fn new(replies: Vec<Result<HttpAnswer, ProbeError>>) -> (Self, RequestLog) {
        let requests = RequestLog::new();
        (
            Self {
                requests: requests.clone(),
                replies: Mutex::new(VecDeque::from(replies)),
            },
            requests,
        )
    }
}

#[async_trait]
impl HttpTransport for ScriptedHttp {
    async fn send(&self, request: OutboundHttp) -> Result<HttpAnswer, ProbeError> {
        self.requests.record(RecordedRequest {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(answer(200, &[], &[])))
    }
}

fn lookup_request() -> GetTranscriptionJobRequest {
    GetTranscriptionJobRequest {
        transcription_job_name: "job-1".to_owned(),
    }
}

fn assert_signed_post(log: &SignerLog, requests: &RequestLog) {
    let posts = log.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].region, "us-east-1");
    assert_eq!(posts[0].url, "https://transcribe.us-east-1.amazonaws.com/");
    assert_eq!(posts[0].body, r#"{"TranscriptionJobName":"job-1"}"#);
    assert_eq!(
        posts[0].headers,
        vec![
            (
                "Content-Type".to_owned(),
                "application/x-amz-json-1.1".to_owned()
            ),
            (
                "X-Amz-Target".to_owned(),
                "Transcribe.GetTranscriptionJob".to_owned()
            ),
        ]
    );
    let sent = requests.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, HttpMethod::Post);
    assert_eq!(sent[0].url, "https://transcribe.us-east-1.amazonaws.com/");
    assert_eq!(sent[0].body, posts[0].body.as_bytes());
    assert!(sent[0].headers.contains(&(
        "X-Amz-Target".to_owned(),
        "Transcribe.GetTranscriptionJob".to_owned()
    )));
}

#[tokio::test]
async fn lookup_posts_the_signed_get_transcription_job_request() {
    let (signing, log) = signer();
    let (http, requests) = ScriptedHttp::new(vec![Ok(answer(
        200,
        &[],
        br#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED"}}"#,
    ))]);
    let lookup = TranscribeJobLookup::new("us-east-1", signing, http);
    let job = lookup
        .get_transcription_job(&lookup_request())
        .await
        .unwrap()
        .transcription_job
        .unwrap();
    assert_eq!(job.transcription_job_status.as_deref(), Some("COMPLETED"));
    assert_signed_post(&log, &requests);
}

#[tokio::test]
async fn lookup_uses_the_regions_partition_suffix() {
    let (signing, log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Ok(answer(200, &[], b"{}"))]);
    let lookup = TranscribeJobLookup::new("cn-north-1", signing, http);
    let _result = lookup.get_transcription_job(&lookup_request()).await;
    assert_eq!(
        log.posts()[0].url,
        "https://transcribe.cn-north-1.amazonaws.com.cn/"
    );
}

#[tokio::test]
async fn lookup_rejects_a_non_2xx_answer_with_its_error_type() {
    let (signing, _log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Ok(answer(
        400,
        &[],
        br#"{"__type":"com.amazon.coral.service#BadRequestException"}"#,
    ))]);
    let lookup = TranscribeJobLookup::new("us-east-1", signing, http);
    match lookup.get_transcription_job(&lookup_request()).await {
        Err(JobLookupError::Rejected {
            status,
            aws_error_type,
        }) => {
            assert_eq!(status, 400);
            assert_eq!(aws_error_type.as_deref(), Some("BadRequestException"));
        }
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("the rejected lookup returned a job"),
    }
}

#[tokio::test]
async fn lookup_maps_a_malformed_success_body_to_invalid_response() {
    let (signing, _log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Ok(answer(200, &[], b"not json"))]);
    let lookup = TranscribeJobLookup::new("us-east-1", signing, http);
    assert!(matches!(
        lookup.get_transcription_job(&lookup_request()).await,
        Err(JobLookupError::InvalidResponse(_))
    ));
}

#[rstest]
#[case::at_the_cap(0x0010_0000, true)]
#[case::one_byte_over(0x0010_0001, false)]
#[tokio::test]
async fn lookup_reads_a_body_up_to_one_mebibyte(#[case] length: usize, #[case] accepted: bool) {
    let mut body = br#"{"TranscriptionJob":{"TranscriptionJobStatus":"COMPLETED"}}"#.to_vec();
    body.resize(length, b' ');
    let (signing, _log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Ok(answer(200, &[], &body))]);
    let lookup = TranscribeJobLookup::new("us-east-1", signing, http);
    let result = lookup.get_transcription_job(&lookup_request()).await;
    assert_eq!(result.is_ok(), accepted);
}

#[test]
fn lookup_header_pairs_keep_each_name_and_value_text() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-opaque", HeaderValue::from_bytes(&[0xFF]).unwrap());
    let pairs = header_pairs(&headers);
    assert_eq!(pairs.len(), 2);
    assert!(pairs.contains(&("content-type".to_owned(), "application/json".to_owned())));
    assert!(pairs.contains(&("x-opaque".to_owned(), String::new())));
}

#[test]
fn lookup_signed_headers_reject_an_invalid_name_or_value() {
    let valid = signed_headers(vec![("X-Amz-Target".to_owned(), "target".to_owned())]).unwrap();
    assert_eq!(valid.get("x-amz-target").unwrap(), "target");
    assert!(matches!(
        signed_headers(vec![("bad name".to_owned(), "value".to_owned())]),
        Err(ProbeError::HeaderName { .. })
    ));
    assert!(matches!(
        signed_headers(vec![("name".to_owned(), "a\nb".to_owned())]),
        Err(ProbeError::HeaderValue { .. })
    ));
}

#[tokio::test]
async fn lookup_maps_a_transport_failure_to_unavailable() {
    let (signing, _log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Err(ProbeError::TaskClosed)]);
    let lookup = TranscribeJobLookup::new("us-east-1", signing, http);
    assert!(matches!(
        lookup.get_transcription_job(&lookup_request()).await,
        Err(JobLookupError::Unavailable)
    ));
}

#[tokio::test]
async fn lookup_maps_a_signing_failure_to_unavailable() {
    let failing = RecordingSigner {
        log: SignerLog::new(),
        fail: true,
    };
    let (http, _requests) = ScriptedHttp::new(Vec::new());
    let lookup = TranscribeJobLookup::new("us-east-1", failing, http);
    assert!(matches!(
        lookup.get_transcription_job(&lookup_request()).await,
        Err(JobLookupError::Unavailable)
    ));
}

fn probe_for(
    region: &str,
    signing: RecordingSigner,
    http: ScriptedHttp,
) -> S3MediaProbe<RecordingSigner, ScriptedHttp> {
    S3MediaProbe::new(region, signing, http, Arc::new(Semaphore::new(1)))
}

fn media_answer() -> HttpAnswer {
    answer(200, &[("last-modified", BEFORE_CREATION)], VALID_WAV)
}

#[tokio::test]
async fn probe_signs_the_s3_get_and_measures_the_object() {
    let (signing, log) = signer();
    let (http, requests) = ScriptedHttp::new(vec![Ok(media_answer())]);
    let probe = probe_for("us-east-1", signing, http);
    let seconds = probe.media_seconds("s3://audio/a.wav", 100.0).await;
    assert_eq!(seconds.unwrap(), Some(Decimal::ONE));
    let gets = log.gets();
    assert_eq!(gets[0].region, "us-east-1");
    assert_eq!(
        gets[0].url,
        "https://audio.s3.us-east-1.amazonaws.com/a.wav"
    );
    let sent = requests.sent();
    assert_eq!(sent[0].method, HttpMethod::Get);
    assert_eq!(
        sent[0].headers,
        vec![(
            "Authorization".to_owned(),
            "AWS4-HMAC-SHA256 signed".to_owned()
        )]
    );
}

#[test]
fn probe_signed_get_signs_the_url_for_the_s3_service() {
    let credentials = Credentials {
        access_key_id: SecretString::from("AKIDEXAMPLE"),
        secret_access_key: SecretString::from("secret"),
        session_token: None,
    };
    let url = "https://audio.s3.us-east-1.amazonaws.com/a.wav";
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let signed = signed_get(&credentials, "us-east-1", url, now).unwrap();
    assert_eq!(signed.method, "GET");
    assert_eq!(signed.url, url);
    assert!(signed.body.is_empty());
    assert!(
        signed
            .headers
            .contains(&("X-Amz-Date".to_owned(), "20231114T221320Z".to_owned()))
    );
    let authorization = signed
        .headers
        .iter()
        .find(|(name, _)| name == "Authorization")
        .unwrap();
    assert!(
        authorization.1.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-east-1/s3/aws4_request"
        )
    );
}

#[tokio::test]
async fn probe_errors_on_a_non_2xx_answer() {
    let (signing, _log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Ok(answer(403, &[], &[]))]);
    let probe = probe_for("us-east-1", signing, http);
    match probe.media_seconds("s3://audio/a.wav", 100.0).await {
        Err(ProbeError::Status { status, .. }) => assert_eq!(status, 403),
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("the rejected download was measured"),
    }
}

#[tokio::test]
async fn probe_is_unmeasurable_for_a_rewritten_object() {
    let (signing, _log) = signer();
    let (http, requests) = ScriptedHttp::new(vec![Ok(answer(
        200,
        &[("last-modified", AFTER_CREATION)],
        VALID_WAV,
    ))]);
    let probe = probe_for("us-east-1", signing, http);
    assert_eq!(
        probe
            .media_seconds("s3://audio/a.wav", 100.0)
            .await
            .unwrap(),
        None
    );
    assert_eq!(requests.sent().len(), 1);
}

#[tokio::test]
async fn probe_is_unmeasurable_for_an_oversized_declared_length() {
    let (signing, _log) = signer();
    let (http, _requests) = ScriptedHttp::new(vec![Ok(answer(
        200,
        &[
            ("last-modified", BEFORE_CREATION),
            ("content-length", "2147483649"),
        ],
        VALID_WAV,
    ))]);
    let probe = probe_for("us-east-1", signing, http);
    assert_eq!(
        probe
            .media_seconds("s3://audio/a.wav", 100.0)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn probe_is_unmeasurable_for_a_foreign_host() {
    let (signing, log) = signer();
    let (http, requests) = ScriptedHttp::new(Vec::new());
    let probe = probe_for("us-east-1", signing, http);
    assert_eq!(
        probe
            .media_seconds("https://files.example.org/a.wav", 100.0)
            .await
            .unwrap(),
        None
    );
    assert_eq!(log.gets().len(), 0);
    assert_eq!(requests.sent().len(), 0);
}

#[tokio::test]
async fn probe_stops_a_stream_past_the_size_cap() {
    let chunks = vec![vec![0_u8; 8], vec![0_u8; 8]];
    let result = download_and_measure(
        stream_of(chunks),
        "s3://audio/a.wav",
        MediaSizeGuard::from(5),
    )
    .await;
    assert_eq!(result.unwrap(), None);
}

#[tokio::test]
async fn probe_holds_a_download_slot_while_it_reads() {
    let (signing, _log) = signer();
    let (http, requests) = ScriptedHttp::new(vec![Ok(media_answer()), Ok(media_answer())]);
    let slots = Arc::new(Semaphore::new(1));
    let held = Arc::clone(&slots).acquire_owned().await.unwrap();
    let probe = S3MediaProbe::new("us-east-1", signing, http, Arc::clone(&slots));
    let pending = timeout(
        Duration::from_millis(50),
        probe.media_seconds("s3://audio/a.wav", 100.0),
    )
    .await;
    let _elapsed = pending.unwrap_err();
    assert_eq!(requests.sent().len(), 0);
    drop(held);
    assert_eq!(
        probe
            .media_seconds("s3://audio/a.wav", 100.0)
            .await
            .unwrap(),
        Some(Decimal::ONE)
    );
}

enum LookupReply {
    Body(&'static str),
    Missing,
    Unavailable,
}

struct FixedLookup {
    reply: LookupReply,
    calls: Mutex<usize>,
}

impl FixedLookup {
    fn new(reply: LookupReply) -> Self {
        Self {
            reply,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl JobLookup for FixedLookup {
    fn get_transcription_job<'lookup>(
        &'lookup self,
        _request: &'lookup GetTranscriptionJobRequest,
    ) -> JobLookupFuture<'lookup> {
        Box::pin(async move {
            let mut calls = self.calls.lock().unwrap();
            *calls = calls.saturating_add(1);
            drop(calls);
            match self.reply {
                LookupReply::Body(body) => {
                    serde_json::from_str(body).map_err(JobLookupError::InvalidResponse)
                }
                LookupReply::Missing => Err(JobLookupError::Rejected {
                    status: 400,
                    aws_error_type: Some("NotFoundException".to_owned()),
                }),
                LookupReply::Unavailable => Err(JobLookupError::Unavailable),
            }
        })
    }
}

struct ScriptedProbe {
    replies: Mutex<VecDeque<Result<Option<Decimal>, ProbeError>>>,
}

impl ScriptedProbe {
    fn new(replies: Vec<Result<Option<Decimal>, ProbeError>>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
        }
    }
}

#[async_trait]
impl MediaDurationProbe for ScriptedProbe {
    async fn media_seconds(
        &self,
        _media_uri: &str,
        _job_created_at: f64,
    ) -> Result<Option<Decimal>, ProbeError> {
        self.replies.lock().unwrap().pop_front().unwrap_or(Ok(None))
    }
}

struct QuietSleeper;

#[async_trait]
impl PollSleeper for QuietSleeper {
    async fn sleep(&self, _seconds: f64) {}
}

fn plan(rate: Decimal, attempts: usize) -> JobPricing<'static> {
    JobPricing {
        job_name: "job-1",
        cost_per_second: rate,
        max_attempts: attempts,
        started_job: None,
    }
}

fn rate() -> Decimal {
    Decimal::new(2, 3)
}

#[tokio::test]
async fn fail_closed_returns_a_measured_cost_unchanged() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::new(15, 1)))]);
    let sleeper = QuietSleeper;
    let cost = price_fail_closed(plan(rate(), 1), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::new(4, 3));
}

#[tokio::test]
async fn fail_closed_charges_the_maximum_when_the_measured_cost_overflows() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITH_MEDIA));
    let probe = ScriptedProbe::new(vec![Ok(Some(Decimal::MAX))]);
    let sleeper = QuietSleeper;
    let per_second = Decimal::from(1_000_u32);
    let expected = Decimal::from(28_800_u32).checked_mul(per_second).unwrap();
    let cost = price_fail_closed(plan(per_second, 1), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, expected);
}

#[tokio::test]
async fn fail_closed_reports_an_error_when_the_max_cost_overflows() {
    let lookup = FixedLookup::new(LookupReply::Body(COMPLETED_WITHOUT_MEDIA));
    let probe = ScriptedProbe::new(Vec::new());
    let sleeper = QuietSleeper;
    match price_fail_closed(plan(Decimal::MAX, 1), &lookup, &probe, &sleeper).await {
        Err(PricingError::Cost(TranscribeCostError::Overflow)) => {}
        Ok(cost) => panic!("the overflowing pricing returned {cost}"),
    }
}

#[tokio::test]
async fn fail_closed_charges_the_maximum_for_a_deleted_job_without_a_started_job() {
    let lookup = FixedLookup::new(LookupReply::Missing);
    let probe = ScriptedProbe::new(Vec::new());
    let sleeper = QuietSleeper;
    let cost = price_fail_closed(plan(rate(), 1), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::new(576, 1));
}

#[tokio::test]
async fn fail_closed_retries_an_unavailable_job_lookup() {
    let lookup = FixedLookup::new(LookupReply::Unavailable);
    let probe = ScriptedProbe::new(Vec::new());
    let sleeper = QuietSleeper;
    let cost = price_fail_closed(plan(rate(), 2), &lookup, &probe, &sleeper)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::new(576, 1));
    assert_eq!(lookup.calls(), 2);
}

struct StaticCredentials;

#[async_trait]
impl CredentialSource for StaticCredentials {
    async fn credentials(&self, _region: &str) -> Result<Credentials, ProbeError> {
        Ok(Credentials {
            access_key_id: SecretString::from("AKIDEXAMPLE"),
            secret_access_key: SecretString::from("secret"),
            session_token: None,
        })
    }
}

// each reading is an hour after the last, so a signature built from a stored instant shows
fn hourly_signer() -> ClockedSigner<StaticCredentials, impl Fn() -> SystemTime + Send + Sync> {
    let readings = AtomicU64::new(0);
    ClockedSigner {
        credentials: StaticCredentials,
        clock: move || {
            let hours = readings.fetch_add(1, Ordering::SeqCst);
            let seconds = hours.saturating_mul(3600).saturating_add(1_700_000_000);
            UNIX_EPOCH
                .checked_add(Duration::from_secs(seconds))
                .unwrap()
        },
    }
}

fn signing_date(request: &RecordedRequest) -> &str {
    let (_name, date) = request
        .headers
        .iter()
        .find(|(name, _)| name == "X-Amz-Date")
        .unwrap();
    date
}

#[tokio::test]
async fn lookup_signs_each_poll_with_the_clock_reading_at_send_time() {
    let (http, requests) = ScriptedHttp::new(Vec::new());
    let lookup = TranscribeJobLookup::new("us-east-1", hourly_signer(), http);
    let _first = lookup.get_transcription_job(&lookup_request()).await;
    let _second = lookup.get_transcription_job(&lookup_request()).await;
    let sent = requests.sent();
    assert_eq!(signing_date(&sent[0]), "20231114T221320Z");
    assert_eq!(signing_date(&sent[1]), "20231114T231320Z");
}

#[tokio::test]
async fn probe_signs_each_download_with_the_clock_reading_at_send_time() {
    let (http, requests) = ScriptedHttp::new(vec![Ok(media_answer()), Ok(media_answer())]);
    let probe = S3MediaProbe::new(
        "us-east-1",
        hourly_signer(),
        http,
        Arc::new(Semaphore::new(1)),
    );
    let _first = probe.media_seconds("s3://audio/a.wav", 100.0).await;
    let _second = probe.media_seconds("s3://audio/a.wav", 100.0).await;
    let sent = requests.sent();
    assert_eq!(signing_date(&sent[0]), "20231114T221320Z");
    assert_eq!(signing_date(&sent[1]), "20231114T231320Z");
}
