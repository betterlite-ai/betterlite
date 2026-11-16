use std::future::ready;

use betterlite_logging::prometheus_query::clock::Clock;
use betterlite_logging::prometheus_query::daily_spend::PrometheusDailySpend;
use betterlite_logging::prometheus_query::transport::{QueryRequest, Transport};
use betterlite_logging::prometheus_query::{PrometheusQuery, PrometheusSettings};
use chrono::{DateTime, Utc};

use super::super::prometheus::{DailySpendFailure, DailySpendSource, PrometheusDailySpendSource};
use super::support::decimal;

const END_OF_WINDOW: i64 = 1_724_112_000;
const BODY: &str = r#"{"status":"success","data":{"resultType":"matrix","result":[{"metric":{},"values":[[1724025600,"1.5"],[1724112000,"2"]]}]}}"#;

#[derive(Debug, thiserror::Error)]
#[error("the fake Prometheus server refused the request")]
struct Refused;

struct RefusedTransport;

impl Transport for RefusedTransport {
    type Error = Refused;

    fn get(&self, _request: QueryRequest) -> impl Future<Output = Result<Vec<u8>, Refused>> + Send {
        ready(Err(Refused))
    }
}

struct FixedTransport {
    body: Vec<u8>,
}

impl Transport for FixedTransport {
    type Error = Refused;

    fn get(&self, _request: QueryRequest) -> impl Future<Output = Result<Vec<u8>, Refused>> + Send {
        ready(Ok(self.body.clone()))
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(END_OF_WINDOW, 0).unwrap()
    }
}

fn settings() -> PrometheusSettings {
    PrometheusSettings {
        url: Some("http://prom:9090".to_owned()),
        selected_instance: None,
    }
}

#[tokio::test]
async fn reads_the_daily_spend_from_a_prometheus_response() {
    let source = PrometheusDailySpendSource {
        query: PrometheusQuery {
            settings: settings(),
            transport: FixedTransport {
                body: BODY.as_bytes().to_vec(),
            },
            clock: FixedClock,
        },
    };
    let spend = source.daily_spend(None).await.unwrap();
    assert_eq!(
        spend,
        [
            PrometheusDailySpend {
                date: "2024-08-19T00:00:00+00:00".to_owned(),
                spend: decimal("1.5"),
            },
            PrometheusDailySpend {
                date: "2024-08-20T00:00:00+00:00".to_owned(),
                spend: decimal("2"),
            },
        ]
    );
}

#[tokio::test]
async fn reports_a_prometheus_failure_as_a_query_failure() {
    let source = PrometheusDailySpendSource {
        query: PrometheusQuery {
            settings: settings(),
            transport: RefusedTransport,
            clock: FixedClock,
        },
    };
    let failure = source.daily_spend(Some("hashed")).await.unwrap_err();
    assert!(matches!(failure, DailySpendFailure::Query));
}
