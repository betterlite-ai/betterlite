use std::io::Read as _;
use std::str::FromStr as _;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::{
    Bucket, CollectorKind, PrometheusMetric, prepare_buckets,
};
use betterlite_prometheus::scrape_endpoint::{parse_scrape_request, render_scrape};
use flate2::read::GzDecoder;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Inc,
    Set,
    Observe,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricSpec {
    kind: Kind,
    name: String,
    help: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    buckets: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    metric: usize,
    labels: Vec<String>,
    action: Action,
    value: serde_json::Number,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    metrics: Vec<MetricSpec>,
    #[serde(default)]
    updates: Vec<Update>,
    #[serde(default)]
    accept: String,
    #[serde(default)]
    accept_encoding: String,
    #[serde(default)]
    names: Vec<String>,
}

#[derive(Serialize)]
struct Output {
    content_type: &'static str,
    gzipped: bool,
    lines: Vec<Line>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Line {
    Comment { comment: String },
    Sample { series: String, value: String },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = CollectorRegistry::default();
        let mut metrics = Vec::new();
        for spec in &input.metrics {
            let metric = build(spec)?;
            registry.register(metric.clone()).or_value_error()?;
            metrics.push(metric);
        }
        for update in &input.updates {
            apply(&metrics, update)?;
        }
        let request =
            parse_scrape_request(&input.accept, &input.accept_encoding, input.names.clone());
        let rendered = render_scrape(&registry, &request);
        let body = if request.gzipped {
            gunzip(&rendered)?
        } else {
            rendered
        };
        let text = String::from_utf8(body).or_value_error()?;
        let output = Output {
            content_type: request.content_type,
            gzipped: request.gzipped,
            lines: lines(&text)?,
        };
        serde_json::to_value(output).or_value_error()
    }
}

fn build(spec: &MetricSpec) -> Result<PrometheusMetric, Box<HelperError>> {
    let kind = match spec.kind {
        Kind::Counter => CollectorKind::Counter,
        Kind::Gauge => CollectorKind::Gauge,
        Kind::Histogram => CollectorKind::Histogram,
    };
    let buckets = match spec.kind {
        Kind::Histogram => {
            let bounds: Vec<Bucket> = spec
                .buckets
                .iter()
                .map(|bound| Bucket::Seconds(*bound))
                .collect();
            prepare_buckets(&bounds).or_value_error()?
        }
        Kind::Counter | Kind::Gauge => Vec::new(),
    };
    let label_names: Vec<&str> = spec.labels.iter().map(String::as_str).collect();
    Ok(PrometheusMetric::new(
        kind,
        &spec.name,
        &spec.help,
        &label_names,
        buckets,
    ))
}

fn apply(metrics: &[PrometheusMetric], update: &Update) -> Result<(), Box<HelperError>> {
    let Some(metric) = metrics.get(update.metric) else {
        return Err(value_error(format!("no metric at index {}", update.metric)));
    };
    if update.labels.len() != metric.description.label_names.len() {
        return Err(value_error(format!(
            "metric {} takes {} label values, got {}",
            update.metric,
            metric.description.label_names.len(),
            update.labels.len()
        )));
    }
    let labels: Vec<(&str, &str)> = metric
        .description
        .label_names
        .iter()
        .map(String::as_str)
        .zip(update.labels.iter().map(String::as_str))
        .collect();
    let amount = Decimal::from_str(&update.value.to_string()).or_value_error()?;
    let value = MetricValue::Finite(amount);
    let handle = metric.labels(&labels);
    match update.action {
        Action::Inc => handle.inc(value),
        Action::Set => handle.set(value),
        Action::Observe => handle.observe(value),
    }
    Ok(())
}

fn gunzip(body: &[u8]) -> Result<Vec<u8>, Box<HelperError>> {
    let mut decoder = GzDecoder::new(body);
    let mut plain = Vec::new();
    decoder.read_to_end(&mut plain).or_value_error()?;
    Ok(plain)
}

fn lines(text: &str) -> Result<Vec<Line>, Box<HelperError>> {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            lines.push(Line::Comment {
                comment: line.to_owned(),
            });
            continue;
        }
        let Some((series, value)) = line.rsplit_once(' ') else {
            return Err(value_error(format!("sample line without a value: {line}")));
        };
        lines.push(Line::Sample {
            series: series.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(lines)
}

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
