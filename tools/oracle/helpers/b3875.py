import math
import sys
from collections.abc import Callable, Sequence
from contextlib import redirect_stdout
from io import StringIO
from types import ModuleType
from typing import Final, Literal, TypedDict

import litellm
import prometheus_client
from litellm.integrations.prometheus import PrometheusLogger
from litellm.types.integrations.prometheus import PrometheusMetricsConfig

INF: Final = math.inf
DEFAULT_BUCKETS: Final[tuple[float, ...]] = tuple(prometheus_client.Histogram.DEFAULT_BUCKETS)
LOGGER_CLASS: Final[Callable[[], PrometheusLogger]] = PrometheusLogger


class MetricsConfig(TypedDict):
    group: str
    metrics: list[str]
    include_labels: list[str] | None


class Input(TypedDict):
    prometheus_metrics_config: list[MetricsConfig]
    prometheus_exclude_metrics: list[str]
    prometheus_exclude_labels: list[str]
    prometheus_latency_buckets: list[float] | None
    prometheus_emit_input_sequence_length_label: bool  # ast-grep-ignore: no-long-names
    caller_identity: Literal["api_key_alias", "user_email", "both"]


class CollectorRecord(TypedDict):
    name: str
    kind: str
    help: str
    labelnames: list[str]
    buckets: list[float | str]


class Listing(TypedDict):
    collectors: list[CollectorRecord]


RECORDED: Final[list[CollectorRecord]] = []


def registered_name(kind: str, name: str) -> str:
    if kind == "counter" and name.endswith("_total"):
        return name.removesuffix("_total")
    return name


def bucket_value(bound: float) -> float | str:
    if math.isinf(bound):
        return "+Inf"
    return bound


def prepared_buckets(buckets: Sequence[float | str]) -> list[float]:
    prepared = [float(bound) for bound in buckets]
    if not prepared or prepared[-1] != INF:
        prepared.append(INF)
    return prepared


def record(
    kind: str,
    name: str,
    documentation: str,
    labelnames: Sequence[str],
    buckets: Sequence[float],
) -> None:
    entry: CollectorRecord = {
        "name": registered_name(kind, name),
        "kind": kind,
        "help": documentation,
        "labelnames": list(labelnames),
        "buckets": [bucket_value(bound) for bound in buckets],
    }
    RECORDED.append(entry)


def recording_counter(
    name: str, documentation: str, labelnames: Sequence[str] = ()
) -> prometheus_client.Counter:
    metric = prometheus_client.Counter(name, documentation, labelnames)
    record("counter", name, documentation, labelnames, ())
    return metric


def recording_gauge(
    name: str, documentation: str, labelnames: Sequence[str] = ()
) -> prometheus_client.Gauge:
    metric = prometheus_client.Gauge(name, documentation, labelnames)
    record("gauge", name, documentation, labelnames, ())
    return metric


def recording_histogram(
    name: str,
    documentation: str,
    labelnames: Sequence[str] = (),
    buckets: Sequence[float | str] = DEFAULT_BUCKETS,
) -> prometheus_client.Histogram:
    metric = prometheus_client.Histogram(name, documentation, labelnames, buckets=buckets)
    record("histogram", name, documentation, labelnames, prepared_buckets(buckets))
    return metric


class MetricsModule(ModuleType):
    Counter: Callable[[str, str, Sequence[str]], prometheus_client.Counter]
    Gauge: Callable[[str, str, Sequence[str]], prometheus_client.Gauge]
    Histogram: Callable[
        [str, str, Sequence[str], Sequence[float | str]], prometheus_client.Histogram
    ]


def metrics_module() -> MetricsModule:
    module = MetricsModule("prometheus_client")
    module.Counter = recording_counter
    module.Gauge = recording_gauge
    module.Histogram = recording_histogram
    return module


def run(input_value: Input) -> Listing:
    RECORDED.clear()
    litellm.prometheus_metrics_config = [
        PrometheusMetricsConfig(
            group=config["group"],
            metrics=config["metrics"],
            include_labels=config["include_labels"],
        )
        for config in input_value["prometheus_metrics_config"]
    ]
    litellm.prometheus_exclude_metrics = input_value["prometheus_exclude_metrics"]
    litellm.prometheus_exclude_labels = input_value["prometheus_exclude_labels"]
    litellm.prometheus_latency_buckets = input_value["prometheus_latency_buckets"]
    litellm.prometheus_emit_input_sequence_length_label = input_value[
        "prometheus_emit_input_sequence_length_label"
    ]
    litellm.prometheus_deployment_and_latency_caller_identity = input_value["caller_identity"]
    # the constructor imports the metric classes itself, so the shim goes in first
    sys.modules["prometheus_client"] = metrics_module()
    # the constructor prints its config tables through rich; stdout carries only the helper's JSON
    with redirect_stdout(StringIO()):
        LOGGER_CLASS()
    return {"collectors": RECORDED}
