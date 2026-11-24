import gzip
from dataclasses import dataclass
from typing import Literal, TypedDict

from litellm.integrations.prometheus_metrics_endpoint import (
    parse_scrape_request,
    render_scrape,
)
from prometheus_client import CollectorRegistry, Counter, Gauge, Histogram


class MetricSpec(TypedDict):
    kind: Literal["counter", "gauge", "histogram"]
    name: str
    help: str
    labels: list[str]
    buckets: list[float]


class Update(TypedDict):
    metric: int
    labels: list[str]
    action: Literal["inc", "set", "observe"]
    value: float


class Input(TypedDict):
    metrics: list[MetricSpec]
    updates: list[Update]
    accept: str
    accept_encoding: str
    names: list[str]


class CommentLine(TypedDict):
    comment: str


class SampleLine(TypedDict):
    series: str
    value: str


class Output(TypedDict):
    content_type: str
    gzipped: bool
    lines: list[CommentLine | SampleLine]


@dataclass(frozen=True)
class Metrics:
    registry: CollectorRegistry
    counters: dict[int, Counter]
    gauges: dict[int, Gauge]
    histograms: dict[int, Histogram]


def build_metrics(input_value: Input) -> Metrics:
    registry = CollectorRegistry()
    metrics = Metrics(registry, {}, {}, {})
    for index, spec in enumerate(input_value["metrics"]):
        match spec["kind"]:
            case "counter":
                metrics.counters[index] = Counter(
                    spec["name"], spec["help"], spec["labels"], registry=registry
                )
            case "gauge":
                metrics.gauges[index] = Gauge(
                    spec["name"], spec["help"], spec["labels"], registry=registry
                )
            case "histogram":
                metrics.histograms[index] = Histogram(
                    spec["name"],
                    spec["help"],
                    spec["labels"],
                    registry=registry,
                    buckets=spec["buckets"],
                )
    return metrics


def apply_updates(input_value: Input, metrics: Metrics) -> None:
    for update in input_value["updates"]:
        index = update["metric"]
        match input_value["metrics"][index]["kind"]:
            case "counter":
                apply_counter(metrics.counters[index], update)
            case "gauge":
                apply_gauge(metrics.gauges[index], update)
            case "histogram":
                apply_histogram(metrics.histograms[index], update)


def apply_counter(counter: Counter, update: Update) -> None:
    if update["labels"]:
        counter.labels(*update["labels"]).inc(update["value"])
    else:
        counter.inc(update["value"])


def apply_gauge(gauge: Gauge, update: Update) -> None:
    if update["labels"]:
        gauge.labels(*update["labels"]).set(update["value"])
    else:
        gauge.set(update["value"])


def apply_histogram(histogram: Histogram, update: Update) -> None:
    if update["labels"]:
        histogram.labels(*update["labels"]).observe(update["value"])
    else:
        histogram.observe(update["value"])


def scrape_lines(text: str) -> list[CommentLine | SampleLine]:
    parts = text.split("\n")
    return [scrape_line(part) for part in parts if part]


def scrape_line(line: str) -> CommentLine | SampleLine:
    if line.startswith("#"):
        return {"comment": line}
    series, _, value = line.rpartition(" ")
    return {"series": series, "value": value}


def run(input_value: Input) -> Output:
    metrics = build_metrics(input_value)
    apply_updates(input_value, metrics)
    scrape = parse_scrape_request(
        accept=input_value["accept"],
        accept_encoding=input_value["accept_encoding"],
        metric_names=tuple(input_value["names"]),
    )
    body = render_scrape(metrics.registry, scrape)
    text = gzip.decompress(body).decode("utf-8") if scrape.gzipped else body.decode("utf-8")
    return {
        "content_type": scrape.content_type,
        "gzipped": scrape.gzipped,
        "lines": scrape_lines(text),
    }
