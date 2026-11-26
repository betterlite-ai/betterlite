mod answer;
mod metrics;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::CollectorKind;
use betterlite_prometheus::collector_registry::metric::gauge_mode::GaugeMode;
use betterlite_prometheus::multiprocess::writer::MultiProcessWriter;
use serde::Deserialize;
use tempfile::TempDir;

use answer::{files_answer, scrape_answer};
use metrics::{apply_updates, build};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Call {
    Files,
    Scrape,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    const fn collector_kind(self) -> CollectorKind {
        match self {
            Self::Counter => CollectorKind::Counter,
            Self::Gauge => CollectorKind::Gauge,
            Self::Histogram => CollectorKind::Histogram,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    All,
    LiveSum,
    LiveMax,
}

impl Mode {
    const fn gauge_mode(self) -> GaugeMode {
        match self {
            Self::All => GaugeMode::All,
            Self::LiveSum => GaugeMode::LiveSum,
            Self::LiveMax => GaugeMode::LiveMax,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Inc,
    Set,
    Observe,
    Remove,
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
    #[serde(default)]
    mode: Mode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateSpec {
    metric: usize,
    #[serde(default)]
    labels: Vec<String>,
    action: Action,
    #[serde(default)]
    value: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pid: u32,
    metrics: Vec<MetricSpec>,
    #[serde(default)]
    updates: Vec<UpdateSpec>,
    call: Call,
    #[serde(default)]
    accept: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let directory = TempDir::new().or_value_error()?;
        let writer =
            MultiProcessWriter::new(directory.path().to_path_buf(), input.pid).or_value_error()?;
        let registry = CollectorRegistry::multiprocess(writer);
        let metrics = build(&registry, &input.metrics)?;
        apply_updates(&metrics, &input.updates)?;
        match input.call {
            Call::Files => files_answer(directory.path()),
            Call::Scrape => scrape_answer(directory.path(), &input.accept),
        }
    }
}
