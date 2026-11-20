import json
import time
from functools import partial
from typing import Literal, TypedDict

import litellm
from litellm.integrations.prometheus import PrometheusLogger
from litellm.integrations.prometheus_helpers.bounded_prometheus_series_tracker import (
    BoundedPrometheusSeriesTracker,
)
from litellm.types.integrations.prometheus import (
    DEFINED_PROMETHEUS_METRICS,
    UserAPIKeyLabelValues,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LABELS: TypeAdapter[UserAPIKeyLabelValues] = TypeAdapter(UserAPIKeyLabelValues)

Outcome = Literal["removed", "absent", "unsupported", "failed"]


class TrackStep(TypedDict):
    metric: str
    labels: dict[str, str | None]
    at_seconds: float
    max_series: int | None
    ttl_seconds: float | None
    cleanup_interval_seconds: float | None


class EmissionStep(TypedDict):
    values: dict[str, str | None]
    at_seconds: float


class Input(TypedDict):
    function: Literal["track_end_user", "inc_labeled_counter"]
    metric_name: DEFINED_PROMETHEUS_METRICS
    supported_labels: list[str]
    metrics: dict[str, Outcome]
    steps: list[TrackStep]
    enable_end_user_tracking: bool
    max_series: int | None
    ttl_seconds: float | None
    cleanup_interval_seconds: float | None
    amount: int
    emissions: list[EmissionStep]


class _ProbeMetric:
    def __init__(self, name: str, outcome: Outcome, log: list[JsonValue]) -> None:
        self._name = name
        self._outcome: Outcome = outcome
        self._log = log
        self._labels: dict[str, JsonValue] = {}

    def labels(self, **label_values: JsonValue) -> "_ProbeMetric":
        child = _ProbeMetric(self._name, self._outcome, self._log)
        child._labels = dict(label_values)
        return child

    def inc(self, amount: float) -> None:
        entry: JsonValue = {
            "event": "increment",
            "metric": self._name,
            "labels": dict(self._labels),
            "amount": str(amount),
        }
        self._log.append(entry)

    def remove(self, *label_values: JsonValue) -> None:
        entry: JsonValue = {
            "event": "removal",
            "metric": self._name,
            "labels": list(label_values),
        }
        self._log.append(entry)
        if self._outcome == "removed":
            return
        if self._outcome == "absent":
            raise KeyError(self._name)
        if self._outcome == "unsupported":
            raise ValueError(self._name)
        raise RuntimeError(self._name)


class ProbeLogger(PrometheusLogger):
    def __init__(self, supported_labels: list[str]) -> None:
        self._supported_labels = supported_labels
        self._bounded_prometheus_series_tracker = BoundedPrometheusSeriesTracker()

    @override
    def get_labels_for_metric(self, metric_name: DEFINED_PROMETHEUS_METRICS) -> list[str]:
        return self._supported_labels

    def track(
        self,
        metric: _ProbeMetric,
        metric_name: DEFINED_PROMETHEUS_METRICS,
        labels: dict[str, str | None],
    ) -> None:
        self._track_end_user_metric_series(metric, metric_name, labels)

    def emit(
        self,
        counter: _ProbeMetric,
        metric_name: DEFINED_PROMETHEUS_METRICS,
        values: dict[str, str | None],
        amount: int,
    ) -> None:
        enum_values = LABELS.validate_json(json.dumps(values))
        self._inc_labeled_counter(counter, metric_name, enum_values, amount=amount)


def set_limits(max_series: int | None, ttl: float | None, cleanup: float | None) -> None:
    litellm.prometheus_end_user_metrics_max_series_per_metric = max_series
    litellm.prometheus_end_user_metrics_ttl_seconds = ttl
    litellm.prometheus_end_user_metrics_cleanup_interval_seconds = cleanup


def probe_metrics(outcomes: dict[str, Outcome], log: list[JsonValue]) -> dict[str, _ProbeMetric]:
    probes: dict[str, _ProbeMetric] = {}
    for name, outcome in outcomes.items():
        probes[name] = _ProbeMetric(name, outcome, log)
    return probes


def track_end_user(call: Input) -> JsonValue:
    logger = ProbeLogger(call["supported_labels"])
    log: list[JsonValue] = []
    probes = probe_metrics(call["metrics"], log)
    for step in call["steps"]:
        set_limits(step["max_series"], step["ttl_seconds"], step["cleanup_interval_seconds"])
        time.monotonic = partial(float, step["at_seconds"])
        logger.track(probes[step["metric"]], call["metric_name"], step["labels"])
    return {"events": log}


def inc_labeled_counter(call: Input) -> JsonValue:
    logger = ProbeLogger(call["supported_labels"])
    log: list[JsonValue] = []
    counter = _ProbeMetric(call["metric_name"], "removed", log)
    set_limits(call["max_series"], call["ttl_seconds"], call["cleanup_interval_seconds"])
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    for step in call["emissions"]:
        time.monotonic = partial(float, step["at_seconds"])
        logger.emit(counter, call["metric_name"], step["values"], call["amount"])
    return {"events": log}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "track_end_user":
        return track_end_user(input_value)
    return inc_labeled_counter(input_value)
