#![cfg(test)]

use std::collections::VecDeque;
use std::fs;
use std::mem::take;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use chrono::{NaiveDate, NaiveTime};
use http::header::AUTHORIZATION;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::{
    BillingError, BillingHttpGet, BillingHttpRequest, BillingHttpResponse, BillingTransportError,
    OPENAI_COSTS_PAGE_LIMIT, OPENAI_ORGANIZATION_COSTS_URL, OpenAiCostCurrency,
    decode_openai_costs_page, fetch_openai_daily_costs,
};

struct FakeHttpGet {
    replies: Mutex<VecDeque<Result<BillingHttpResponse, BillingTransportError>>>,
    requests: Mutex<Vec<BillingHttpRequest>>,
}

impl FakeHttpGet {
    fn with(replies: Vec<Result<BillingHttpResponse, BillingTransportError>>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take_requests(&self) -> Vec<BillingHttpRequest> {
        let mut requests = self.requests.lock().unwrap();
        take(&mut *requests)
    }
}

#[async_trait]
impl BillingHttpGet for FakeHttpGet {
    async fn get_billing(
        &self,
        request: BillingHttpRequest,
    ) -> Result<BillingHttpResponse, BillingTransportError> {
        self.requests.lock().unwrap().push(request);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted billing reply")
    }
}

#[derive(Serialize)]
struct PageFixture {
    data: Vec<BucketFixture>,
    has_more: bool,
    next_page: Option<String>,
}

#[derive(Serialize)]
struct BucketFixture {
    start_time: i64,
    results: Vec<ResultFixture>,
}

#[derive(Serialize)]
struct ResultFixture {
    amount: AmountFixture,
}

#[derive(Serialize)]
struct AmountFixture {
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    value: Decimal,
    currency: &'static str,
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn midnight(text: &str) -> i64 {
    date(text).and_time(NaiveTime::MIN).and_utc().timestamp()
}

fn bucket(day: &str, amounts: &[&str]) -> BucketFixture {
    BucketFixture {
        start_time: midnight(day),
        results: amounts
            .iter()
            .map(|amount| ResultFixture {
                amount: AmountFixture {
                    value: dec(amount),
                    currency: "usd",
                },
            })
            .collect(),
    }
}

fn page(buckets: Vec<BucketFixture>, has_more: bool, next_page: Option<&str>) -> String {
    serde_json::to_string(&PageFixture {
        data: buckets,
        has_more,
        next_page: next_page.map(str::to_owned),
    })
    .unwrap()
}

fn ok(body: String) -> BillingHttpResponse {
    BillingHttpResponse {
        status: 200,
        body: body.into_bytes(),
    }
}

fn key() -> SecretString {
    SecretString::from("sk-admin".to_owned())
}

fn no_dates() -> &'static [String] {
    let none: &'static [String] = &[];
    none
}

#[test]
fn decodes_a_costs_page_with_defaults() {
    let page = decode_openai_costs_page(br#"{"data":[{"start_time":1762732800}]}"#).unwrap();
    assert_eq!(page.data.len(), 1);
    assert!(page.data[0].results.is_empty());
    assert!(!page.has_more);
    assert_eq!(page.next_page, None);
}

#[test]
fn decodes_usd_amounts_and_ignores_extra_fields() {
    let page = decode_openai_costs_page(
        br#"{"object":"page","data":[{"object":"bucket","start_time":1762732800,"end_time":1762819200,"results":[{"object":"organization.costs.result","amount":{"value":1.25,"currency":"usd"},"line_item":"x"}]}],"has_more":true,"next_page":"cursor"}"#,
    )
    .unwrap();
    assert_eq!(page.data[0].start_time, 1_762_732_800);
    assert_eq!(page.data[0].results.len(), 1);
    assert_eq!(page.data[0].results[0].amount.value, dec("1.25"));
    assert_eq!(
        page.data[0].results[0].amount.currency,
        OpenAiCostCurrency::Usd
    );
    assert!(page.has_more);
    assert_eq!(page.next_page.as_deref(), Some("cursor"));
}

#[rstest]
#[case::non_usd_currency(
    br#"{"data":[{"start_time":0,"results":[{"amount":{"value":1.0,"currency":"eur"}}]}]}"#
)]
#[case::missing_data(br#"{"has_more":false}"#)]
#[case::missing_amount(br#"{"data":[{"start_time":0,"results":[{}]}]}"#)]
#[case::non_numeric_amount(
    br#"{"data":[{"start_time":0,"results":[{"amount":{"value":true,"currency":"usd"}}]}]}"#
)]
#[case::fractional_start_time(br#"{"data":[{"start_time":1.5,"results":[]}]}"#)]
#[case::truncated(br#"{"data":["#)]
fn rejects_a_malformed_costs_page(#[case] body: &[u8]) {
    assert!(matches!(
        decode_openai_costs_page(body),
        Err(BillingError::Shape { .. })
    ));
}

#[tokio::test]
async fn aggregates_buckets_by_utc_day_across_pages() {
    let first = page(
        vec![
            bucket("2026-11-10", &["1.5"]),
            bucket("2026-11-10", &["2.25"]),
        ],
        true,
        Some("page-2"),
    );
    let second = page(
        vec![
            bucket("2026-11-10", &["0.25"]),
            bucket("2026-11-12", &["4"]),
        ],
        false,
        None,
    );
    let http = FakeHttpGet::with(vec![Ok(ok(first)), Ok(ok(second))]);
    let days = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-12"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap();
    assert_eq!(days.len(), 2);
    assert_eq!(days.get("2026-11-10"), Some(&dec("4")));
    assert_eq!(days.get("2026-11-12"), Some(&dec("4")));
    assert_eq!(http.take_requests().len(), 2);
}

#[tokio::test]
async fn queries_the_fixed_window_and_repeats_project_ids() {
    let http = FakeHttpGet::with(vec![Ok(ok(page(Vec::new(), false, None)))]);
    let projects = vec!["proj_a".to_owned(), "proj_b".to_owned()];
    let days = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-12"),
        &key(),
        &projects,
        &http,
    )
    .await
    .unwrap();
    assert!(days.is_empty());
    let requests = http.take_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, OPENAI_ORGANIZATION_COSTS_URL);
    assert_eq!(
        request.query,
        vec![
            ("start_time".to_owned(), midnight("2026-11-10").to_string()),
            ("end_time".to_owned(), midnight("2026-11-13").to_string()),
            ("bucket_width".to_owned(), "1d".to_owned()),
            ("limit".to_owned(), OPENAI_COSTS_PAGE_LIMIT.to_string()),
            ("project_ids[]".to_owned(), "proj_a".to_owned()),
            ("project_ids[]".to_owned(), "proj_b".to_owned()),
        ]
    );
    assert_eq!(request.headers.len(), 1);
    let authorization = request.headers.get(AUTHORIZATION).unwrap();
    assert_eq!(authorization, "Bearer sk-admin");
    assert!(authorization.is_sensitive());
}

#[tokio::test]
async fn omits_project_ids_when_none_are_configured() {
    let http = FakeHttpGet::with(vec![Ok(ok(page(Vec::new(), false, None)))]);
    fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-12"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap();
    let requests = http.take_requests();
    assert_eq!(requests[0].query.len(), 4);
    assert!(
        requests[0]
            .query
            .iter()
            .all(|(name, _value)| name != "project_ids[]")
    );
}

#[tokio::test]
async fn appends_the_page_cursor_after_the_window() {
    let first = page(vec![bucket("2026-11-10", &["1"])], true, Some("cursor-1"));
    let second = page(vec![bucket("2026-11-11", &["2"])], false, None);
    let http = FakeHttpGet::with(vec![Ok(ok(first)), Ok(ok(second))]);
    fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-12"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap();
    let requests = http.take_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].query.len(), 5);
    assert_eq!(
        requests[1].query.last(),
        Some(&("page".to_owned(), "cursor-1".to_owned()))
    );
}

#[tokio::test]
async fn stops_when_has_more_is_false_even_with_a_cursor() {
    let single = page(vec![bucket("2026-11-10", &["1"])], false, Some("ignored"));
    let http = FakeHttpGet::with(vec![Ok(ok(single))]);
    let days = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap();
    assert_eq!(days.get("2026-11-10"), Some(&dec("1")));
    assert_eq!(http.take_requests().len(), 1);
}

#[tokio::test]
async fn stops_when_has_more_has_no_cursor() {
    let single = page(vec![bucket("2026-11-10", &["1"])], true, None);
    let http = FakeHttpGet::with(vec![Ok(ok(single))]);
    fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap();
    assert_eq!(http.take_requests().len(), 1);
}

#[tokio::test]
async fn rejects_a_repeated_pagination_cursor() {
    let first = page(vec![bucket("2026-11-10", &["1"])], true, Some("same"));
    let second = page(vec![bucket("2026-11-10", &["1"])], true, Some("same"));
    let http = FakeHttpGet::with(vec![Ok(ok(first)), Ok(ok(second))]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        BillingError::RepeatedCursor { cursor } if cursor == "same"
    ));
    assert_eq!(http.take_requests().len(), 2);
}

#[tokio::test]
async fn reports_a_network_failure_without_the_admin_key() {
    let http = FakeHttpGet::with(vec![Err(BillingTransportError::Transport(
        TransportError::Closed,
    ))]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BillingError::Request { .. }));
    let detail = error.to_string();
    assert!(detail.starts_with("request failed: "), "{detail}");
    assert!(!detail.contains("sk-admin"), "{detail}");
}

#[tokio::test]
async fn reports_a_non_200_status_with_the_first_300_characters() {
    let body = "\u{e9}".repeat(301);
    let http = FakeHttpGet::with(vec![Ok(BillingHttpResponse {
        status: 500,
        body: body.into_bytes(),
    })]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    let BillingError::HttpStatus { status, detail } = error else {
        panic!("expected a status failure");
    };
    assert_eq!(status, 500);
    assert_eq!(detail.chars().count(), 300);
    assert!(detail.chars().all(|character| character == '\u{e9}'));
}

#[tokio::test]
async fn rejects_a_non_200_success_lookalike() {
    let http = FakeHttpGet::with(vec![Ok(BillingHttpResponse {
        status: 201,
        body: b"{}".to_vec(),
    })]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        BillingError::HttpStatus { status: 201, .. }
    ));
}

#[tokio::test]
async fn rejects_a_non_utf8_error_body() {
    let http = FakeHttpGet::with(vec![Ok(BillingHttpResponse {
        status: 400,
        body: vec![0xff, 0xfe],
    })]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BillingError::BodyText { .. }));
}

#[tokio::test]
async fn rejects_a_malformed_success_body() {
    let http = FakeHttpGet::with(vec![Ok(ok("{".to_owned()))]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BillingError::Shape { .. }));
}

#[tokio::test]
async fn rejects_a_window_past_the_calendar_before_any_request() {
    let http = FakeHttpGet::with(Vec::new());
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        NaiveDate::MAX,
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BillingError::DateRange));
    assert!(http.take_requests().is_empty());
}

#[tokio::test]
async fn rejects_a_bucket_outside_the_calendar() {
    let http = FakeHttpGet::with(vec![Ok(ok(page(
        vec![BucketFixture {
            start_time: i64::MAX,
            results: Vec::new(),
        }],
        false,
        None,
    )))]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        BillingError::BucketDate { start_time } if start_time == i64::MAX
    ));
}

#[tokio::test]
async fn rejects_an_overflowing_day_total() {
    let maximum = "79228162514264337593543950335";
    let http = FakeHttpGet::with(vec![Ok(ok(page(
        vec![
            bucket("2026-11-10", &[maximum]),
            bucket("2026-11-10", &[maximum]),
        ],
        false,
        None,
    )))]);
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &key(),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BillingError::AmountOverflow));
}

#[tokio::test]
async fn rejects_an_admin_key_that_cannot_be_a_header() {
    let http = FakeHttpGet::with(Vec::new());
    let error = fetch_openai_daily_costs(
        date("2026-11-10"),
        date("2026-11-10"),
        &SecretString::from("sk-\nadmin".to_owned()),
        no_dates(),
        &http,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, BillingError::Header { .. }));
    assert!(!error.to_string().contains("sk-"), "{error}");
    assert!(http.take_requests().is_empty());
}

#[test]
fn exposes_the_admin_key_nowhere_in_the_page_types() {
    let page = decode_openai_costs_page(
        br#"{"data":[{"start_time":1762732800,"results":[{"amount":{"value":1,"currency":"usd"}}]}]}"#,
    )
    .unwrap();
    assert!(!format!("{page:?}").contains("sk-"));
    let key = key();
    assert_eq!(key.expose_secret(), "sk-admin");
}

#[rstest]
fn replays_the_openai_costs_page_corpus(#[files("fuzz/corpus/openai_costs_page/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _decoded = decode_openai_costs_page(&bytes);
}
