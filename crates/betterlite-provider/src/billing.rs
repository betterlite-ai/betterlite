use std::collections::{BTreeMap, BTreeSet};
use std::str::Utf8Error;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use chrono::{Days, NaiveDate, NaiveTime};
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue, InvalidHeaderValue};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::{ParseError, Url};

mod tests;

pub const OPENAI_ADMIN_KEY_ENV_VAR: &str = "OPENAI_ADMIN_KEY";
pub const OPENAI_ORGANIZATION_COSTS_URL: &str = "https://api.openai.com/v1/organization/costs";
pub const OPENAI_COSTS_PAGE_LIMIT: i64 = 180;
pub const BILLING_TIMEOUT_SECONDS: Duration = Duration::from_secs(30);
const MAX_DETAIL_CHARS: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum BillingTransportError {
    #[error("the billing URL is invalid")]
    Url(#[source] ParseError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

pub struct BillingHttpRequest {
    pub url: &'static str,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
}

pub struct BillingHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait BillingHttpGet: Send + Sync {
    async fn get_billing(
        &self,
        request: BillingHttpRequest,
    ) -> Result<BillingHttpResponse, BillingTransportError>;
}

#[async_trait]
impl BillingHttpGet for ProxiedClient {
    async fn get_billing(
        &self,
        request: BillingHttpRequest,
    ) -> Result<BillingHttpResponse, BillingTransportError> {
        let url = Url::parse(request.url).map_err(BillingTransportError::Url)?;
        let response = self
            .send_buffered(OutboundRequest {
                method: Method::GET,
                url,
                query: request.query,
                headers: request.headers,
                body: RequestBody::Empty,
                deadline: BILLING_TIMEOUT_SECONDS,
            })
            .await?;
        Ok(BillingHttpResponse {
            status: response.status,
            body: response.body,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("request failed: {source}")]
    Request {
        #[source]
        source: BillingTransportError,
    },
    #[error("HTTP {status}: {detail}")]
    HttpStatus { status: u16, detail: String },
    #[error("the response body is not UTF-8")]
    BodyText {
        #[source]
        source: Utf8Error,
    },
    #[error("unexpected response shape: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the admin key cannot be carried in an Authorization header")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the costs page repeated the pagination cursor {cursor}")]
    RepeatedCursor { cursor: String },
    #[error("the costs page names the start time {start_time} outside the calendar")]
    BucketDate { start_time: i64 },
    #[error("the billing window cannot be represented on the calendar")]
    DateRange,
    #[error("the bill total is too large to represent exactly")]
    AmountOverflow,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiCostsPage {
    pub data: Vec<OpenAiCostBucket>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub next_page: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiCostBucket {
    pub start_time: i64,
    #[serde(default)]
    pub results: Vec<OpenAiCostResult>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiCostResult {
    pub amount: OpenAiCostAmount,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiCostAmount {
    #[serde(deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize")]
    pub value: Decimal,
    pub currency: OpenAiCostCurrency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiCostCurrency {
    Usd,
}

pub fn decode_openai_costs_page(body: &[u8]) -> Result<OpenAiCostsPage, BillingError> {
    serde_json::from_slice(body).map_err(|source| BillingError::Shape { source })
}

pub async fn fetch_openai_daily_costs(
    start_date: NaiveDate,
    end_date: NaiveDate,
    admin_key: &SecretString,
    project_ids: &[String],
    http_get: &dyn BillingHttpGet,
) -> Result<BTreeMap<String, Decimal>, BillingError> {
    let end_exclusive = end_date
        .checked_add_days(Days::new(1))
        .ok_or(BillingError::DateRange)?;
    let mut window = vec![
        (
            "start_time".to_owned(),
            start_date
                .and_time(NaiveTime::MIN)
                .and_utc()
                .timestamp()
                .to_string(),
        ),
        (
            "end_time".to_owned(),
            end_exclusive
                .and_time(NaiveTime::MIN)
                .and_utc()
                .timestamp()
                .to_string(),
        ),
        ("bucket_width".to_owned(), "1d".to_owned()),
        ("limit".to_owned(), OPENAI_COSTS_PAGE_LIMIT.to_string()),
    ];
    for project_id in project_ids {
        window.push(("project_ids[]".to_owned(), project_id.clone()));
    }
    let mut authorization = HeaderValue::from_str(&format!("Bearer {}", admin_key.expose_secret()))
        .map_err(|source| BillingError::Header { source })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    let mut page: Option<String> = None;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut buckets: Vec<OpenAiCostBucket> = Vec::new();
    loop {
        let mut query = window.clone();
        if let Some(cursor) = &page {
            query.push(("page".to_owned(), cursor.clone()));
        }
        let response = http_get
            .get_billing(BillingHttpRequest {
                url: OPENAI_ORGANIZATION_COSTS_URL,
                query,
                headers: headers.clone(),
            })
            .await
            .map_err(|source| BillingError::Request { source })?;
        if response.status != 200 {
            return Err(http_status_error(response.status, &response.body));
        }
        let decoded = decode_openai_costs_page(&response.body)?;
        buckets.extend(decoded.data);
        if !decoded.has_more {
            break;
        }
        let Some(cursor) = decoded.next_page else {
            break;
        };
        if !seen.insert(cursor.clone()) {
            return Err(BillingError::RepeatedCursor { cursor });
        }
        page = Some(cursor);
    }
    let mut days: BTreeMap<String, Decimal> = BTreeMap::new();
    for bucket in &buckets {
        let day = bucket_day(bucket.start_time)?;
        let mut bucket_total = Decimal::ZERO;
        for result in &bucket.results {
            bucket_total = bucket_total
                .checked_add(result.amount.value)
                .ok_or(BillingError::AmountOverflow)?;
        }
        let entry = days.entry(day).or_insert(Decimal::ZERO);
        *entry = entry
            .checked_add(bucket_total)
            .ok_or(BillingError::AmountOverflow)?;
    }
    Ok(days)
}

fn bucket_day(start_time: i64) -> Result<String, BillingError> {
    chrono::DateTime::from_timestamp(start_time, 0)
        .map(|moment| moment.date_naive().format("%Y-%m-%d").to_string())
        .ok_or(BillingError::BucketDate { start_time })
}

fn http_status_error(status: u16, body: &[u8]) -> BillingError {
    match std::str::from_utf8(body) {
        Ok(text) => BillingError::HttpStatus {
            status,
            detail: text.chars().take(MAX_DETAIL_CHARS).collect(),
        },
        Err(source) => BillingError::BodyText { source },
    }
}
